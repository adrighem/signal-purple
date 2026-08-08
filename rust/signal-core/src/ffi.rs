// SPDX-License-Identifier: AGPL-3.0-only
use std::ffi::{CStr, c_char};
use std::os::fd::RawFd;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use tokio::sync::{mpsc as tokio_mpsc, watch};

const BACKEND_THREAD_STACK_BYTES: usize = 8 * 1024 * 1024;

use crate::acknowledgment::AcknowledgmentInbox;
use crate::attachment::{AttachmentAdmission, AttachmentAdmissionError, MAX_ATTACHMENT_BYTES};
use crate::backend::{self, Command, Config, StorePassphrase, WorkerContext};
#[cfg(test)]
use crate::event::Event;
use crate::event::{self, ABI_VERSION, OwnedEvent, SignalEvent};
#[cfg(test)]
use crate::event_queue::EventSink;
use crate::event_queue::{EventPoll, EventQueue, event_queue};

const MAX_STORE_PATH_BYTES: usize = 4096;
const MAX_DEVICE_NAME_BYTES: usize = 128;
const MAX_PASSPHRASE_BYTES: usize = 4096;
const MAX_RECIPIENT_BYTES: usize = 256;
const GROUP_KEY_BYTES: usize = 64;
const MAX_MESSAGE_BYTES: usize = 64 * 1024;
const MAX_ATTACHMENT_FILENAME_BYTES: usize = 255;
const MAX_CONTENT_TYPE_BYTES: usize = 255;
const EVENT_QUEUE_CAPACITY: usize = 4096;

const ABI_CONTRACT_VALUE_COUNT: usize = 64;
const ABI_CONTRACT_VALUES: [i64; ABI_CONTRACT_VALUE_COUNT] = [
    ABI_VERSION as i64,
    SignalStatus::Ok as i64,
    SignalStatus::InvalidArgument as i64,
    SignalStatus::NotReady as i64,
    SignalStatus::QueueFull as i64,
    SignalStatus::InternalError as i64,
    event::EVENT_LINK_QR as i64,
    event::EVENT_READY as i64,
    event::EVENT_CONTACT as i64,
    event::EVENT_GROUP as i64,
    event::EVENT_MESSAGE as i64,
    event::EVENT_GROUP_MESSAGE as i64,
    event::EVENT_TYPING as i64,
    event::EVENT_RECEIPT as i64,
    event::EVENT_NOTICE_RESERVED as i64,
    event::EVENT_ERROR as i64,
    event::EVENT_DISCONNECTED as i64,
    event::EVENT_CONTACT_SYNC_BEGIN as i64,
    event::EVENT_CONTACT_SYNC_END as i64,
    event::EVENT_GROUP_SYNC_BEGIN as i64,
    event::EVENT_GROUP_SYNC_END as i64,
    event::EVENT_GROUP_MEMBER as i64,
    event::EVENT_IDENTITY_CHANGE as i64,
    event::EVENT_IDENTITY_ACCEPTED as i64,
    event::EVENT_ATTACHMENT as i64,
    event::EVENT_ATTACHMENT_SENT as i64,
    event::EVENT_GROUP_LEFT as i64,
    event::EVENT_RECOVERING as i64,
    event::EVENT_ACCOUNT as i64,
    0,
    event::FLAG_OUTGOING as i64,
    event::FLAG_FATAL as i64,
    event::FLAG_TRANSIENT as i64,
    std::mem::size_of::<SignalCoreConfig>() as i64,
    std::mem::align_of::<SignalCoreConfig>() as i64,
    std::mem::offset_of!(SignalCoreConfig, abi_version) as i64,
    std::mem::offset_of!(SignalCoreConfig, struct_size) as i64,
    std::mem::offset_of!(SignalCoreConfig, store_path) as i64,
    std::mem::offset_of!(SignalCoreConfig, device_name) as i64,
    std::mem::offset_of!(SignalCoreConfig, passphrase) as i64,
    std::mem::size_of::<SignalEvent>() as i64,
    std::mem::align_of::<SignalEvent>() as i64,
    std::mem::offset_of!(SignalEvent, abi_version) as i64,
    std::mem::offset_of!(SignalEvent, struct_size) as i64,
    std::mem::offset_of!(SignalEvent, kind) as i64,
    std::mem::offset_of!(SignalEvent, flags) as i64,
    std::mem::offset_of!(SignalEvent, request_id) as i64,
    std::mem::offset_of!(SignalEvent, timestamp_ms) as i64,
    std::mem::offset_of!(SignalEvent, value) as i64,
    std::mem::offset_of!(SignalEvent, peer_id) as i64,
    std::mem::offset_of!(SignalEvent, chat_id) as i64,
    std::mem::offset_of!(SignalEvent, title) as i64,
    std::mem::offset_of!(SignalEvent, text) as i64,
    std::mem::offset_of!(SignalEvent, data) as i64,
    std::mem::offset_of!(SignalEvent, data_len) as i64,
    MAX_STORE_PATH_BYTES as i64,
    MAX_DEVICE_NAME_BYTES as i64,
    MAX_PASSPHRASE_BYTES as i64,
    MAX_RECIPIENT_BYTES as i64,
    GROUP_KEY_BYTES as i64,
    MAX_MESSAGE_BYTES as i64,
    MAX_ATTACHMENT_FILENAME_BYTES as i64,
    MAX_CONTENT_TYPE_BYTES as i64,
    MAX_ATTACHMENT_BYTES as i64,
];

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignalStatus {
    Ok = 0,
    InvalidArgument = -1,
    NotReady = -2,
    QueueFull = -3,
    InternalError = -4,
}

#[repr(C)]
pub struct SignalCoreConfig {
    abi_version: u32,
    struct_size: u32,
    store_path: *const c_char,
    device_name: *const c_char,
    passphrase: *const c_char,
}

pub struct SignalCore {
    commands: tokio_mpsc::Sender<Command>,
    acknowledgments: Arc<AcknowledgmentInbox>,
    shutdown: watch::Sender<bool>,
    events: EventQueue,
    ready: Arc<AtomicBool>,
    attachments: Arc<AttachmentAdmission>,
    join: Mutex<Option<JoinHandle<()>>>,
}

fn ffi_guard(operation: impl FnOnce() -> SignalStatus) -> SignalStatus {
    catch_unwind(AssertUnwindSafe(operation)).unwrap_or(SignalStatus::InternalError)
}

macro_rules! status_try {
    ($expression:expr) => {
        match $expression {
            Ok(value) => value,
            Err(status) => return status,
        }
    };
}

unsafe fn required_string(
    value: *const c_char,
    maximum_bytes: usize,
) -> Result<String, SignalStatus> {
    if value.is_null() {
        return Err(SignalStatus::InvalidArgument);
    }
    // SAFETY: the caller promises a NUL-terminated C string for ABI string
    // arguments. The bytes are copied before this function returns.
    let bytes = unsafe { CStr::from_ptr(value) }.to_bytes();
    if bytes.is_empty() || bytes.len() > maximum_bytes {
        return Err(SignalStatus::InvalidArgument);
    }
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| SignalStatus::InvalidArgument)
}

unsafe fn required_store_passphrase(
    value: *const c_char,
    maximum_bytes: usize,
) -> Result<StorePassphrase, SignalStatus> {
    // SAFETY: this helper has the same C-string contract as `required_string`.
    unsafe { required_string(value, maximum_bytes) }.map(StorePassphrase::new)
}

fn queue_command(core: &SignalCore, command: Command) -> SignalStatus {
    if !core.ready.load(Ordering::Acquire) {
        return SignalStatus::NotReady;
    }
    match core.commands.try_send(command) {
        Ok(()) => SignalStatus::Ok,
        Err(tokio_mpsc::error::TrySendError::Full(_)) => SignalStatus::QueueFull,
        Err(tokio_mpsc::error::TrySendError::Closed(_)) => SignalStatus::InternalError,
    }
}

fn queue_control_command(core: &SignalCore, command: Command) -> SignalStatus {
    match core.commands.try_send(command) {
        Ok(()) => SignalStatus::Ok,
        Err(tokio_mpsc::error::TrySendError::Full(_)) => SignalStatus::QueueFull,
        Err(tokio_mpsc::error::TrySendError::Closed(_)) => SignalStatus::InternalError,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn signal_core_abi_version() -> u32 {
    ABI_VERSION
}

#[unsafe(no_mangle)]
pub extern "C" fn signal_core_abi_contract_value(index: u32) -> i64 {
    ABI_CONTRACT_VALUES
        .get(index as usize)
        .copied()
        .unwrap_or(i64::MIN)
}

#[unsafe(no_mangle)]
/// Returns the borrowed file descriptor which becomes readable when an event
/// is queued, or `-1` for an invalid core.
///
/// # Safety
///
/// `core` must be null or point to a live core. The descriptor remains owned
/// by the core and must not be closed by the caller.
pub unsafe extern "C" fn signal_core_event_fd(core: *const SignalCore) -> RawFd {
    catch_unwind(AssertUnwindSafe(|| {
        if core.is_null() {
            return -1;
        }
        // SAFETY: checked above; the caller keeps the core live while using
        // the borrowed descriptor.
        unsafe { &*core }.events.event_fd()
    }))
    .unwrap_or(-1)
}

#[unsafe(no_mangle)]
/// Creates a backend core and transfers its allocation through `out_core`.
///
/// # Safety
///
/// `out_core` must be writable and `config` must point to its advertised
/// prefix. Every non-null string field must be valid NUL-terminated UTF-8.
pub unsafe extern "C" fn signal_core_new(
    config: *const SignalCoreConfig,
    out_core: *mut *mut SignalCore,
) -> SignalStatus {
    ffi_guard(|| {
        if out_core.is_null() {
            return SignalStatus::InvalidArgument;
        }
        // SAFETY: `out_core` was validated above and remains owned by C.
        unsafe { *out_core = std::ptr::null_mut() };
        if config.is_null() {
            return SignalStatus::InvalidArgument;
        }
        // SAFETY: every config version begins with these two u32 fields. Read
        // that prefix before creating a reference to the full current struct.
        let abi_version = unsafe { std::ptr::addr_of!((*config).abi_version).read_unaligned() };
        // SAFETY: same validated version prefix as above.
        let struct_size = unsafe { std::ptr::addr_of!((*config).struct_size).read_unaligned() };
        if abi_version != ABI_VERSION
            || struct_size < size_of::<SignalCoreConfig>() as u32
            || !(config as usize).is_multiple_of(align_of::<SignalCoreConfig>())
        {
            return SignalStatus::InvalidArgument;
        }
        // SAFETY: the checked size covers the full struct, and the caller owns
        // it for the duration of this call.
        let config = unsafe { &*config };

        // SAFETY: validated and copied by `required_string`.
        let store_path =
            status_try!(unsafe { required_string(config.store_path, MAX_STORE_PATH_BYTES) });
        status_try!(
            backend::ensure_store_parent(&store_path).map_err(|_| SignalStatus::InternalError)
        );
        // SAFETY: validated and copied by `required_string`.
        let device_name =
            status_try!(unsafe { required_string(config.device_name, MAX_DEVICE_NAME_BYTES) });
        let (command_tx, command_rx) = tokio_mpsc::channel(128);
        let acknowledgments = AcknowledgmentInbox::new();
        let worker_acknowledgments = Arc::clone(&acknowledgments);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (event_sink, events) =
            status_try!(event_queue(EVENT_QUEUE_CAPACITY).map_err(|_| SignalStatus::InternalError));
        let ready = Arc::new(AtomicBool::new(false));
        let worker_ready = Arc::clone(&ready);
        // SAFETY: validated, copied, and immediately put under zeroizing
        // ownership by `required_store_passphrase`.
        let passphrase = status_try!(unsafe {
            required_store_passphrase(config.passphrase, MAX_PASSPHRASE_BYTES)
        });
        let worker_config = Config {
            store_path,
            device_name,
            passphrase,
        };

        let join = status_try!(
            std::thread::Builder::new()
                .name("signal-purple-core".into())
                .stack_size(BACKEND_THREAD_STACK_BYTES)
                .spawn(move || {
                    backend::run_worker(WorkerContext {
                        config: worker_config,
                        commands: command_rx,
                        acknowledgments: worker_acknowledgments,
                        shutdown: shutdown_rx,
                        events: event_sink,
                        ready: worker_ready,
                    });
                })
                .map_err(|_| SignalStatus::InternalError)
        );

        let core = Box::new(SignalCore {
            commands: command_tx,
            acknowledgments,
            shutdown: shutdown_tx,
            events,
            ready,
            attachments: AttachmentAdmission::new(),
            join: Mutex::new(Some(join)),
        });
        // SAFETY: `out_core` was checked above. Ownership transfers to C and
        // must be returned through `signal_core_free`.
        unsafe { *out_core = Box::into_raw(core) };
        SignalStatus::Ok
    })
}

#[unsafe(no_mangle)]
/// Queues a direct message on a live core.
///
/// # Safety
///
/// `core` must be live and exclusively serialized with teardown. `recipient`
/// and `message` must be valid NUL-terminated strings.
pub unsafe extern "C" fn signal_core_send_message(
    core: *mut SignalCore,
    request_id: u64,
    recipient: *const c_char,
    message: *const c_char,
) -> SignalStatus {
    ffi_guard(|| {
        if core.is_null() {
            return SignalStatus::InvalidArgument;
        }
        // SAFETY: copied immediately after validation.
        let recipient = status_try!(unsafe { required_string(recipient, MAX_RECIPIENT_BYTES) });
        // SAFETY: copied immediately after validation.
        let message = status_try!(unsafe { required_string(message, MAX_MESSAGE_BYTES) });
        // SAFETY: `core` is live until `signal_core_free`, which must not race
        // any ABI call.
        queue_command(
            unsafe { &*core },
            Command::SendMessage {
                request_id,
                recipient,
                message,
            },
        )
    })
}

#[unsafe(no_mangle)]
/// Queues a group message on a live core.
///
/// # Safety
///
/// `core` must be live and exclusively serialized with teardown. `group_key`
/// and `message` must be valid NUL-terminated strings.
pub unsafe extern "C" fn signal_core_send_group_message(
    core: *mut SignalCore,
    request_id: u64,
    group_key: *const c_char,
    message: *const c_char,
) -> SignalStatus {
    ffi_guard(|| {
        if core.is_null() {
            return SignalStatus::InvalidArgument;
        }
        // SAFETY: copied immediately after validation.
        let group_key = status_try!(unsafe { required_string(group_key, GROUP_KEY_BYTES) });
        if group_key.len() != GROUP_KEY_BYTES
            || hex::decode(&group_key).map_or(true, |v| v.len() != 32)
        {
            return SignalStatus::InvalidArgument;
        }
        // SAFETY: copied immediately after validation.
        let message = status_try!(unsafe { required_string(message, MAX_MESSAGE_BYTES) });
        // SAFETY: see `signal_core_send_message`.
        queue_command(
            unsafe { &*core },
            Command::SendGroupMessage {
                request_id,
                group_key,
                message,
            },
        )
    })
}

#[unsafe(no_mangle)]
/// Queues a request to leave one synchronized Signal group.
///
/// # Safety
///
/// `core` must be live and exclusively serialized with teardown. `group_key`
/// must be a valid NUL-terminated opaque group identifier.
pub unsafe extern "C" fn signal_core_leave_group(
    core: *mut SignalCore,
    request_id: u64,
    group_key: *const c_char,
) -> SignalStatus {
    ffi_guard(|| {
        if core.is_null() || request_id == 0 {
            return SignalStatus::InvalidArgument;
        }
        // SAFETY: copied immediately after validation.
        let group_key = status_try!(unsafe { required_string(group_key, GROUP_KEY_BYTES) });
        if group_key.len() != GROUP_KEY_BYTES
            || hex::decode(&group_key).map_or(true, |v| v.len() != 32)
        {
            return SignalStatus::InvalidArgument;
        }
        // SAFETY: `core` is live and C serializes this call with teardown.
        queue_command(
            unsafe { &*core },
            Command::LeaveGroup {
                request_id,
                group_key,
            },
        )
    })
}

struct AttachmentInput {
    recipient: *const c_char,
    filename: *const c_char,
    content_type: *const c_char,
    data: *const u8,
    data_len: usize,
}

unsafe fn send_attachment(
    core: *mut SignalCore,
    request_id: u64,
    input: AttachmentInput,
    group: bool,
) -> SignalStatus {
    ffi_guard(|| {
        if core.is_null()
            || input.data.is_null()
            || input.data_len == 0
            || input.data_len > MAX_ATTACHMENT_BYTES
        {
            return SignalStatus::InvalidArgument;
        }
        // SAFETY: `core` remains live and ABI calls are serialized by C.
        let core = unsafe { &*core };
        if !core.ready.load(Ordering::Acquire) {
            return SignalStatus::NotReady;
        }
        // SAFETY: all strings and bytes are copied during this call.
        let recipient =
            status_try!(unsafe { required_string(input.recipient, MAX_RECIPIENT_BYTES) });
        let filename =
            status_try!(unsafe { required_string(input.filename, MAX_ATTACHMENT_FILENAME_BYTES) });
        let content_type =
            status_try!(unsafe { required_string(input.content_type, MAX_CONTENT_TYPE_BYTES) });
        if group
            && (recipient.len() != GROUP_KEY_BYTES
                || hex::decode(&recipient).map_or(true, |value| value.len() != 32))
        {
            return SignalStatus::InvalidArgument;
        }
        let attachment_permit = match core.attachments.try_reserve(request_id, input.data_len) {
            Ok(permit) => permit,
            Err(AttachmentAdmissionError::Invalid) => return SignalStatus::InvalidArgument,
            Err(AttachmentAdmissionError::Capacity) => return SignalStatus::QueueFull,
        };
        let command_slot = match core.commands.try_reserve() {
            Ok(slot) => slot,
            Err(tokio_mpsc::error::TrySendError::Full(_)) => return SignalStatus::QueueFull,
            Err(tokio_mpsc::error::TrySendError::Closed(_)) => {
                return SignalStatus::InternalError;
            }
        };
        // SAFETY: the caller guarantees `data_len` readable bytes; the bound
        // above prevents an oversized allocation, and the bytes are copied.
        let data = unsafe { std::slice::from_raw_parts(input.data, input.data_len) }.to_vec();
        command_slot.send(Command::SendAttachment {
            request_id,
            recipient,
            filename,
            content_type,
            data,
            group,
            permit: attachment_permit,
        });
        SignalStatus::Ok
    })
}

#[unsafe(no_mangle)]
/// Queues one bounded attachment for a direct Signal recipient.
///
/// # Safety
///
/// All pointers must remain readable for this call. `data` must address
/// `data_len` bytes. The core must be live and serialized with teardown.
pub unsafe extern "C" fn signal_core_send_attachment(
    core: *mut SignalCore,
    request_id: u64,
    recipient: *const c_char,
    filename: *const c_char,
    content_type: *const c_char,
    data: *const u8,
    data_len: usize,
) -> SignalStatus {
    // SAFETY: this function has the same pointer contract as the helper.
    unsafe {
        send_attachment(
            core,
            request_id,
            AttachmentInput {
                recipient,
                filename,
                content_type,
                data,
                data_len,
            },
            false,
        )
    }
}

#[unsafe(no_mangle)]
/// Queues one bounded attachment for a synchronized Signal group.
///
/// # Safety
///
/// All pointers must remain readable for this call. `data` must address
/// `data_len` bytes. The core must be live and serialized with teardown.
pub unsafe extern "C" fn signal_core_send_group_attachment(
    core: *mut SignalCore,
    request_id: u64,
    group_key: *const c_char,
    filename: *const c_char,
    content_type: *const c_char,
    data: *const u8,
    data_len: usize,
) -> SignalStatus {
    // SAFETY: this function has the same pointer contract as the helper.
    unsafe {
        send_attachment(
            core,
            request_id,
            AttachmentInput {
                recipient: group_key,
                filename,
                content_type,
                data,
                data_len,
            },
            true,
        )
    }
}

#[unsafe(no_mangle)]
/// Cancels an in-flight attachment upload when it has not completed yet.
///
/// # Safety
///
/// `core` must be live and serialized with teardown.
pub unsafe extern "C" fn signal_core_cancel_attachment(
    core: *mut SignalCore,
    request_id: u64,
) -> SignalStatus {
    ffi_guard(|| {
        if core.is_null() || request_id == 0 {
            return SignalStatus::InvalidArgument;
        }
        // SAFETY: `core` remains live and ABI calls are serialized by C.
        let _ = unsafe { &*core }.attachments.cancel(request_id);
        SignalStatus::Ok
    })
}

#[unsafe(no_mangle)]
/// Queues a direct-chat typing state on a live core.
///
/// # Safety
///
/// `core` must be live and exclusively serialized with teardown. `recipient`
/// must be a valid NUL-terminated string.
pub unsafe extern "C" fn signal_core_set_typing(
    core: *mut SignalCore,
    request_id: u64,
    recipient: *const c_char,
    typing: i32,
) -> SignalStatus {
    ffi_guard(|| {
        if core.is_null() {
            return SignalStatus::InvalidArgument;
        }
        // SAFETY: copied immediately after validation.
        let recipient = status_try!(unsafe { required_string(recipient, MAX_RECIPIENT_BYTES) });
        // SAFETY: see `signal_core_send_message`.
        queue_command(
            unsafe { &*core },
            Command::SetTyping {
                request_id,
                recipient,
                typing: typing != 0,
            },
        )
    })
}

#[unsafe(no_mangle)]
/// Queues durable acknowledgment of a message accepted by Purple.
///
/// # Safety
///
/// `core` must be live and exclusively serialized with teardown.
pub unsafe extern "C" fn signal_core_ack_message(
    core: *mut SignalCore,
    delivery_id: u64,
) -> SignalStatus {
    ffi_guard(|| {
        if core.is_null() || delivery_id == 0 {
            return SignalStatus::InvalidArgument;
        }
        // SAFETY: `core` is live until `signal_core_free`, which must not race
        // any ABI call.
        if unsafe { &*core }.acknowledgments.submit(delivery_id) {
            SignalStatus::Ok
        } else {
            SignalStatus::InternalError
        }
    })
}

#[unsafe(no_mangle)]
/// Accepts a pending identity replacement for one canonical recipient.
///
/// # Safety
///
/// `core` must be live and `recipient` must be a valid NUL-terminated string.
pub unsafe extern "C" fn signal_core_accept_identity(
    core: *mut SignalCore,
    request_id: u64,
    recipient: *const c_char,
) -> SignalStatus {
    ffi_guard(|| {
        if core.is_null() {
            return SignalStatus::InvalidArgument;
        }
        // SAFETY: copied immediately after validation.
        let recipient = status_try!(unsafe { required_string(recipient, MAX_RECIPIENT_BYTES) });
        queue_control_command(
            unsafe { &*core },
            Command::AcceptIdentity {
                request_id,
                recipient,
            },
        )
    })
}

#[unsafe(no_mangle)]
/// Dismisses a non-blocking identity replacement notice.
///
/// # Safety
///
/// `core` must be live and `recipient` must be a valid NUL-terminated string.
pub unsafe extern "C" fn signal_core_dismiss_identity(
    core: *mut SignalCore,
    request_id: u64,
    recipient: *const c_char,
) -> SignalStatus {
    ffi_guard(|| {
        if core.is_null() {
            return SignalStatus::InvalidArgument;
        }
        // SAFETY: copied immediately after validation.
        let recipient = status_try!(unsafe { required_string(recipient, MAX_RECIPIENT_BYTES) });
        queue_control_command(
            unsafe { &*core },
            Command::DismissIdentity {
                request_id,
                recipient,
            },
        )
    })
}

#[unsafe(no_mangle)]
/// Queues a read receipt after Purple reports that a conversation is focused.
///
/// # Safety
///
/// `core` must be live and `recipient` must be a valid NUL-terminated string.
pub unsafe extern "C" fn signal_core_mark_read(
    core: *mut SignalCore,
    request_id: u64,
    recipient: *const c_char,
    timestamp: u64,
) -> SignalStatus {
    ffi_guard(|| {
        if core.is_null() || timestamp == 0 {
            return SignalStatus::InvalidArgument;
        }
        // SAFETY: copied immediately after validation.
        let recipient = status_try!(unsafe { required_string(recipient, MAX_RECIPIENT_BYTES) });
        queue_command(
            unsafe { &*core },
            Command::MarkRead {
                request_id,
                recipient,
                timestamp,
            },
        )
    })
}

#[unsafe(no_mangle)]
/// Polls one owned backend event without blocking.
///
/// # Safety
///
/// `core` must be live and serialized with teardown, and `out_event` must be
/// writable. A returned event must be passed exactly once to
/// `signal_event_free`.
pub unsafe extern "C" fn signal_core_poll_event(
    core: *mut SignalCore,
    out_event: *mut *mut SignalEvent,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        if out_event.is_null() {
            return -1;
        }
        // SAFETY: `out_event` was validated above and remains owned by C.
        unsafe { *out_event = std::ptr::null_mut() };
        if core.is_null() {
            return -1;
        }
        // SAFETY: checked above; C serializes poll/free with core teardown.
        let core = unsafe { &*core };
        match core.events.poll() {
            EventPoll::Event(event) => {
                // SAFETY: checked above; event ownership transfers to C.
                unsafe { *out_event = OwnedEvent::into_raw(event) };
                1
            }
            EventPoll::Empty => 0,
            EventPoll::Disconnected => -1,
        }
    }))
    .unwrap_or(-1)
}

#[unsafe(no_mangle)]
/// Releases an event returned by `signal_core_poll_event`.
///
/// # Safety
///
/// `event` must be null or an allocation returned by the poll function that
/// has not previously been freed.
pub unsafe extern "C" fn signal_event_free(event: *mut SignalEvent) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the pointer came from `signal_core_poll_event` and has not
        // previously been freed.
        unsafe { OwnedEvent::free(event) };
    }));
}

#[unsafe(no_mangle)]
/// Cancels and joins a backend worker. Repeated calls are safe.
///
/// # Safety
///
/// `core` must be null or live, and teardown must have exclusive access with
/// no concurrent polling or command submission.
pub unsafe extern "C" fn signal_core_shutdown(core: *mut SignalCore) {
    if core.is_null() {
        return;
    }
    // SAFETY: caller guarantees exclusive teardown access.
    let core = unsafe { &*core };
    core.ready.store(false, Ordering::Release);
    core.attachments.cancel_all();
    core.events.close();
    let _ = core.shutdown.send(true);
    let join = match core.join.lock() {
        Ok(mut guard) => guard.take(),
        Err(poisoned) => poisoned.into_inner().take(),
    };
    if let Some(join) = join {
        let _ = join.join();
    }
}

#[unsafe(no_mangle)]
/// Shuts down and releases a backend core.
///
/// # Safety
///
/// `core` must be null or a unique allocation returned by `signal_core_new`.
/// No call may race this function and the pointer must not be used afterward.
pub unsafe extern "C" fn signal_core_free(core: *mut SignalCore) {
    if core.is_null() {
        return;
    }
    // SAFETY: caller transfers the unique allocation back to Rust.
    unsafe { signal_core_shutdown(core) };
    // SAFETY: `core` was allocated by `signal_core_new` and shutdown has
    // joined its worker.
    drop(unsafe { Box::from_raw(core) });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn abi_status_values_remain_stable() {
        assert_eq!(SignalStatus::Ok as i32, 0);
        assert_eq!(SignalStatus::InvalidArgument as i32, -1);
        assert_eq!(SignalStatus::NotReady as i32, -2);
        assert_eq!(SignalStatus::QueueFull as i32, -3);
        assert_eq!(SignalStatus::InternalError as i32, -4);
    }

    fn test_core(
        commands: tokio_mpsc::Sender<Command>,
        shutdown: watch::Sender<bool>,
        join: Option<JoinHandle<()>>,
        ready: bool,
    ) -> SignalCore {
        let (_event_sink, events) = event_queue(1).unwrap();
        SignalCore {
            commands,
            acknowledgments: AcknowledgmentInbox::new(),
            shutdown,
            events,
            ready: Arc::new(AtomicBool::new(ready)),
            attachments: AttachmentAdmission::new(),
            join: Mutex::new(join),
        }
    }

    fn event_test_core(capacity: usize) -> (SignalCore, EventSink) {
        let (commands, _command_receiver) = tokio_mpsc::channel(1);
        let (shutdown, _shutdown_receiver) = watch::channel(false);
        let (event_sink, events) = event_queue(capacity).unwrap();
        (
            SignalCore {
                commands,
                acknowledgments: AcknowledgmentInbox::new(),
                shutdown,
                events,
                ready: Arc::new(AtomicBool::new(true)),
                attachments: AttachmentAdmission::new(),
                join: Mutex::new(None),
            },
            event_sink,
        )
    }

    fn enqueue_test_event(sink: &EventSink, event: Event) {
        sink.emit(event);
    }

    #[test]
    fn constructor_clears_output_on_error() {
        let mut output = std::ptr::dangling_mut::<SignalCore>();

        // SAFETY: the output pointer is valid for this call; a null config is
        // an intentionally tested error path.
        let status = unsafe { signal_core_new(std::ptr::null(), &mut output) };

        assert_eq!(status, SignalStatus::InvalidArgument);
        assert!(output.is_null());
    }

    #[test]
    fn required_passphrase_copy_enters_zeroizing_ownership() {
        let input = CString::new("test-store-passphrase").unwrap();
        let dropped = Arc::new(AtomicBool::new(false));

        // SAFETY: `input` is a live, NUL-terminated C string.
        let mut passphrase =
            unsafe { required_store_passphrase(input.as_ptr(), MAX_PASSPHRASE_BYTES) }.unwrap();
        passphrase.observe_drop(Arc::clone(&dropped));

        assert_eq!(passphrase.as_str().len(), input.as_bytes().len());
        drop(passphrase);
        assert!(dropped.load(Ordering::Acquire));
    }

    #[test]
    fn poll_clears_output_on_error() {
        let mut output = std::ptr::dangling_mut::<SignalEvent>();

        // SAFETY: the output pointer is valid for this call; a null core is an
        // intentionally tested error path.
        let result = unsafe { signal_core_poll_event(std::ptr::null_mut(), &mut output) };

        assert_eq!(result, -1);
        assert!(output.is_null());
    }

    #[test]
    fn event_fd_is_borrowed_from_the_live_core() {
        let (core, _sink) = event_test_core(1);

        // SAFETY: `core` remains live for the duration of the call.
        assert_eq!(
            unsafe { signal_core_event_fd(&core) },
            core.events.event_fd()
        );
        // SAFETY: a null core is an explicitly supported error path.
        assert_eq!(unsafe { signal_core_event_fd(std::ptr::null()) }, -1);
    }

    #[test]
    fn exact_event_batch_keeps_one_level_trigger_until_empty() {
        const BATCH_SIZE: usize = 64;
        let (mut core, sink) = event_test_core(BATCH_SIZE);

        for request_id in 1..=BATCH_SIZE as u64 {
            enqueue_test_event(
                &sink,
                Event {
                    kind: crate::event::EVENT_MESSAGE,
                    request_id,
                    ..Event::default()
                },
            );
        }

        for request_id in 1..=BATCH_SIZE as u64 {
            let mut event = std::ptr::null_mut();
            // SAFETY: the core and output pointer remain live for the call.
            assert_eq!(unsafe { signal_core_poll_event(&mut core, &mut event) }, 1);
            assert_eq!(unsafe { (*event).request_id }, request_id);
            // SAFETY: this test uniquely owns the returned event.
            unsafe { signal_event_free(event) };
        }

        let mut event = std::ptr::dangling_mut::<SignalEvent>();
        // SAFETY: the core and output pointer remain live for the call.
        assert_eq!(unsafe { signal_core_poll_event(&mut core, &mut event) }, 0);
        assert!(event.is_null());

        enqueue_test_event(
            &sink,
            Event {
                kind: crate::event::EVENT_MESSAGE,
                request_id: 65,
                ..Event::default()
            },
        );
        assert_eq!(unsafe { signal_core_poll_event(&mut core, &mut event) }, 1);
        assert_eq!(unsafe { (*event).request_id }, 65);
        // SAFETY: this test uniquely owns the returned event.
        unsafe { signal_event_free(event) };
    }

    #[test]
    fn event_backpressure_preserves_order_without_synthesizing_overflow() {
        let (mut core, sink) = event_test_core(1);
        enqueue_test_event(
            &sink,
            Event {
                kind: crate::event::EVENT_MESSAGE,
                request_id: 7,
                ..Event::default()
            },
        );
        let producer_sink = sink.clone();
        let producer = std::thread::spawn(move || {
            enqueue_test_event(
                &producer_sink,
                Event {
                    kind: crate::event::EVENT_MESSAGE,
                    request_id: 8,
                    ..Event::default()
                },
            );
        });

        let mut event = std::ptr::null_mut();
        // SAFETY: the core and output pointer remain live for each call.
        assert_eq!(unsafe { signal_core_poll_event(&mut core, &mut event) }, 1);
        assert_eq!(unsafe { (*event).request_id }, 7);
        // SAFETY: this test uniquely owns the returned event.
        unsafe { signal_event_free(event) };
        producer.join().unwrap();
        // SAFETY: the core and output pointer remain live for each call.
        assert_eq!(unsafe { signal_core_poll_event(&mut core, &mut event) }, 1);
        assert_eq!(unsafe { (*event).request_id }, 8);
        // SAFETY: this test uniquely owns the returned event.
        unsafe { signal_event_free(event) };
        // SAFETY: the core and output pointer remain live for each call.
        assert_eq!(unsafe { signal_core_poll_event(&mut core, &mut event) }, 0);
    }

    #[test]
    fn constructor_rejects_a_truncated_version_prefix() {
        let prefix = [ABI_VERSION, size_of::<[u32; 2]>() as u32];
        let mut output = std::ptr::dangling_mut::<SignalCore>();

        // SAFETY: `signal_core_new` reads only the advertised two-u32 prefix
        // before rejecting its size.
        let status =
            unsafe { signal_core_new(prefix.as_ptr().cast::<SignalCoreConfig>(), &mut output) };

        assert_eq!(status, SignalStatus::InvalidArgument);
        assert!(output.is_null());
    }

    #[test]
    fn command_queue_preserves_order_and_reports_pressure() {
        let (commands, mut receiver) = tokio_mpsc::channel(2);
        let (shutdown, _shutdown_receiver) = watch::channel(false);
        let core = test_core(commands, shutdown, None, true);

        assert_eq!(
            queue_command(
                &core,
                Command::SendMessage {
                    request_id: 1,
                    recipient: "aci:first".into(),
                    message: "one".into(),
                }
            ),
            SignalStatus::Ok
        );
        assert_eq!(
            queue_command(
                &core,
                Command::SetTyping {
                    request_id: 2,
                    recipient: "aci:second".into(),
                    typing: true,
                }
            ),
            SignalStatus::Ok
        );
        assert_eq!(
            queue_command(
                &core,
                Command::SetTyping {
                    request_id: 3,
                    recipient: "aci:third".into(),
                    typing: false,
                }
            ),
            SignalStatus::QueueFull
        );

        assert!(matches!(
            receiver.try_recv(),
            Ok(Command::SendMessage { request_id: 1, .. })
        ));
        assert!(matches!(
            receiver.try_recv(),
            Ok(Command::SetTyping { request_id: 2, .. })
        ));
    }

    #[test]
    fn message_acknowledgments_are_accepted_before_ready() {
        let (commands, _receiver) = tokio_mpsc::channel(1);
        let (shutdown, _shutdown_receiver) = watch::channel(false);
        let mut core = test_core(commands, shutdown, None, false);
        core.acknowledgments.register(42);

        // SAFETY: the core remains live and uniquely owned for this call.
        assert_eq!(
            unsafe { signal_core_ack_message(&mut core, 42) },
            SignalStatus::Ok
        );
        assert_eq!(core.acknowledgments.pending_len(), 1);
    }

    #[test]
    fn message_acknowledgments_do_not_share_bounded_work_capacity() {
        let (commands, _receiver) = tokio_mpsc::channel(1);
        let (shutdown, _shutdown_receiver) = watch::channel(false);
        let mut core = test_core(commands, shutdown, None, true);
        core.acknowledgments.register(42);

        assert_eq!(
            queue_command(
                &core,
                Command::SetTyping {
                    request_id: 1,
                    recipient: "aci:recipient".into(),
                    typing: true,
                }
            ),
            SignalStatus::Ok
        );
        // SAFETY: the core remains live and uniquely owned for this call.
        assert_eq!(
            unsafe { signal_core_ack_message(&mut core, 42) },
            SignalStatus::Ok
        );
        assert_eq!(core.acknowledgments.pending_len(), 1);
    }

    #[test]
    fn attachment_abi_copies_bounded_input() {
        let (commands, mut receiver) = tokio_mpsc::channel(1);
        let (shutdown, _shutdown_receiver) = watch::channel(false);
        let mut core = test_core(commands, shutdown, None, true);
        let recipient = CString::new("aci:recipient").unwrap();
        let filename = CString::new("photo.jpg").unwrap();
        let content_type = CString::new("image/jpeg").unwrap();
        let data = [1u8, 2, 3];

        // SAFETY: all pointers are valid for the duration of the call.
        let status = unsafe {
            signal_core_send_attachment(
                &mut core,
                7,
                recipient.as_ptr(),
                filename.as_ptr(),
                content_type.as_ptr(),
                data.as_ptr(),
                data.len(),
            )
        };

        assert_eq!(status, SignalStatus::Ok);
        assert!(matches!(
            receiver.try_recv(),
            Ok(Command::SendAttachment {
                request_id: 7,
                data: queued,
                group: false,
                ..
            }) if queued == data
        ));
    }

    #[test]
    fn attachment_abi_rejects_invalid_sizes() {
        let (commands, _receiver) = tokio_mpsc::channel(1);
        let (shutdown, _shutdown_receiver) = watch::channel(false);
        let mut core = test_core(commands, shutdown, None, true);
        let value = CString::new("value").unwrap();
        let byte = 0u8;

        // SAFETY: the byte pointer is valid; oversized input is rejected
        // before the function reads it.
        let status = unsafe {
            signal_core_send_attachment(
                &mut core,
                1,
                value.as_ptr(),
                value.as_ptr(),
                value.as_ptr(),
                &byte,
                MAX_ATTACHMENT_BYTES + 1,
            )
        };
        assert_eq!(status, SignalStatus::InvalidArgument);

        // SAFETY: the zero length causes the null data pointer to be rejected.
        let status = unsafe {
            signal_core_send_attachment(
                &mut core,
                1,
                value.as_ptr(),
                value.as_ptr(),
                value.as_ptr(),
                std::ptr::null(),
                0,
            )
        };
        assert_eq!(status, SignalStatus::InvalidArgument);
    }

    #[test]
    fn attachment_abi_bounds_aggregate_payload_and_active_ids() {
        let (commands, mut receiver) = tokio_mpsc::channel(3);
        let (shutdown, _shutdown_receiver) = watch::channel(false);
        let mut core = test_core(commands, shutdown, None, true);
        core.attachments = AttachmentAdmission::for_test(8, 2);
        let value = CString::new("value").unwrap();
        let data = [1u8; 4];

        let send = |core: &mut SignalCore, request_id, data: &[u8]| {
            // SAFETY: the core, strings, and byte slice remain valid for this call.
            unsafe {
                signal_core_send_attachment(
                    core,
                    request_id,
                    value.as_ptr(),
                    value.as_ptr(),
                    value.as_ptr(),
                    data.as_ptr(),
                    data.len(),
                )
            }
        };

        assert_eq!(send(&mut core, 1, &data), SignalStatus::Ok);
        assert_eq!(send(&mut core, 2, &data), SignalStatus::Ok);
        assert_eq!(core.attachments.usage(), (8, 2));
        assert_eq!(send(&mut core, 3, &[1]), SignalStatus::QueueFull);
        assert_eq!(send(&mut core, 1, &[1]), SignalStatus::InvalidArgument);

        drop(receiver.try_recv().unwrap());
        assert_eq!(send(&mut core, 1, &data), SignalStatus::Ok);
    }

    #[test]
    fn attachment_abi_rejects_zero_ids_without_consuming_capacity() {
        let (commands, _receiver) = tokio_mpsc::channel(1);
        let (shutdown, _shutdown_receiver) = watch::channel(false);
        let mut core = test_core(commands, shutdown, None, true);
        core.attachments = AttachmentAdmission::for_test(1, 1);
        let value = CString::new("value").unwrap();
        let data = [1u8];

        // SAFETY: the core, strings, and byte slice remain valid for this call.
        let status = unsafe {
            signal_core_send_attachment(
                &mut core,
                0,
                value.as_ptr(),
                value.as_ptr(),
                value.as_ptr(),
                data.as_ptr(),
                data.len(),
            )
        };

        assert_eq!(status, SignalStatus::InvalidArgument);
        assert_eq!(core.attachments.usage(), (0, 0));
    }

    #[test]
    fn attachment_cancellation_does_not_share_bounded_work_capacity() {
        let (commands, _receiver) = tokio_mpsc::channel(1);
        let (shutdown, _shutdown_receiver) = watch::channel(false);
        let mut core = test_core(commands, shutdown, None, true);
        let value = CString::new("value").unwrap();
        let data = [1u8];

        // SAFETY: the core, strings, and data remain live for both calls.
        assert_eq!(
            unsafe {
                signal_core_send_attachment(
                    &mut core,
                    7,
                    value.as_ptr(),
                    value.as_ptr(),
                    value.as_ptr(),
                    data.as_ptr(),
                    data.len(),
                )
            },
            SignalStatus::Ok
        );
        // SAFETY: the core remains live and uniquely owned for this call.
        assert_eq!(
            unsafe { signal_core_cancel_attachment(&mut core, 7) },
            SignalStatus::Ok
        );
        // SAFETY: cancellation is idempotent while the core remains live.
        assert_eq!(
            unsafe { signal_core_cancel_attachment(&mut core, 7) },
            SignalStatus::Ok
        );
    }

    #[test]
    fn attachment_abi_checks_readiness_and_queue_space_before_admission() {
        let (commands, mut receiver) = tokio_mpsc::channel(1);
        let (shutdown, _shutdown_receiver) = watch::channel(false);
        let mut core = test_core(commands, shutdown, None, false);
        core.attachments = AttachmentAdmission::for_test(1, 1);
        let value = CString::new("value").unwrap();
        let data = [1u8];

        let send = |core: &mut SignalCore, request_id| {
            // SAFETY: the core, strings, and byte slice remain valid for this call.
            unsafe {
                signal_core_send_attachment(
                    core,
                    request_id,
                    value.as_ptr(),
                    value.as_ptr(),
                    value.as_ptr(),
                    data.as_ptr(),
                    data.len(),
                )
            }
        };

        assert_eq!(send(&mut core, 1), SignalStatus::NotReady);
        assert_eq!(core.attachments.usage(), (0, 0));

        core.ready.store(true, Ordering::Release);
        assert_eq!(
            queue_command(
                &core,
                Command::SetTyping {
                    request_id: 1,
                    recipient: "aci:recipient".into(),
                    typing: true,
                },
            ),
            SignalStatus::Ok
        );
        assert_eq!(send(&mut core, 1), SignalStatus::QueueFull);
        assert_eq!(core.attachments.usage(), (0, 0));

        drop(receiver.try_recv().unwrap());
        assert_eq!(send(&mut core, 1), SignalStatus::Ok);
    }

    #[test]
    fn leave_group_abi_validates_and_queues_the_identifier() {
        let (commands, mut receiver) = tokio_mpsc::channel(1);
        let (shutdown, _shutdown_receiver) = watch::channel(false);
        let mut core = test_core(commands, shutdown, None, true);
        let group_id = CString::new("ab".repeat(32)).unwrap();

        // SAFETY: the core and group identifier remain valid for this call.
        let status = unsafe { signal_core_leave_group(&mut core, 23, group_id.as_ptr()) };

        assert_eq!(status, SignalStatus::Ok);
        assert!(matches!(
            receiver.try_recv(),
            Ok(Command::LeaveGroup {
                request_id: 23,
                group_key,
            }) if group_key == "ab".repeat(32)
        ));
    }

    #[test]
    fn leave_group_abi_rejects_missing_request_and_invalid_identifier() {
        let (commands, _receiver) = tokio_mpsc::channel(1);
        let (shutdown, _shutdown_receiver) = watch::channel(false);
        let mut core = test_core(commands, shutdown, None, true);
        let valid = CString::new("ab".repeat(32)).unwrap();
        let invalid = CString::new("not-a-group").unwrap();

        // SAFETY: all pointers are valid for the duration of each call.
        assert_eq!(
            unsafe { signal_core_leave_group(&mut core, 0, valid.as_ptr()) },
            SignalStatus::InvalidArgument
        );
        // SAFETY: all pointers are valid for the duration of each call.
        assert_eq!(
            unsafe { signal_core_leave_group(&mut core, 1, invalid.as_ptr()) },
            SignalStatus::InvalidArgument
        );
    }

    #[test]
    fn shutdown_cancels_and_joins_the_worker() {
        let (commands, _command_receiver) = tokio_mpsc::channel(1);
        let (shutdown, mut shutdown_receiver) = watch::channel(false);
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stopped = Arc::clone(&stopped);
        let join = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap();
            runtime.block_on(async move {
                if !*shutdown_receiver.borrow() {
                    shutdown_receiver.changed().await.unwrap();
                }
                worker_stopped.store(true, Ordering::Release);
            });
        });
        let core = test_core(commands, shutdown, Some(join), true);
        let attachment = core.attachments.try_reserve(7, 1).unwrap();
        let core = Box::into_raw(Box::new(core));

        // SAFETY: this test uniquely owns the core allocation until free.
        unsafe { signal_core_shutdown(core) };
        assert!(stopped.load(Ordering::Acquire));
        assert!(attachment.is_cancelled());
        // SAFETY: shutdown is idempotent and this test uniquely owns `core`.
        unsafe { signal_core_free(core) };
    }

    #[test]
    fn shutdown_unblocks_a_worker_waiting_for_event_capacity() {
        let (commands, _command_receiver) = tokio_mpsc::channel(1);
        let (shutdown, _shutdown_receiver) = watch::channel(false);
        let (sink, events) = event_queue(1).unwrap();
        sink.emit(Event::default());
        let blocked_sink = sink.clone();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (finished_tx, finished_rx) = std::sync::mpsc::channel();
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stopped = Arc::clone(&stopped);
        let join = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            blocked_sink.emit(Event::default());
            worker_stopped.store(true, Ordering::Release);
            finished_tx.send(()).unwrap();
        });
        started_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while events.waiting_producers() == 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "worker did not enter event-queue backpressure"
            );
            std::thread::yield_now();
        }
        assert!(matches!(
            finished_rx.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        let core = Box::into_raw(Box::new(SignalCore {
            commands,
            acknowledgments: AcknowledgmentInbox::new(),
            shutdown,
            events,
            ready: Arc::new(AtomicBool::new(true)),
            attachments: AttachmentAdmission::new(),
            join: Mutex::new(Some(join)),
        }));

        // SAFETY: this test uniquely owns the core allocation until free.
        unsafe { signal_core_shutdown(core) };
        finished_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert!(stopped.load(Ordering::Acquire));
        // SAFETY: shutdown is idempotent and this test uniquely owns `core`.
        unsafe { signal_core_free(core) };
    }
}
