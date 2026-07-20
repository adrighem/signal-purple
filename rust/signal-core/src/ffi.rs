// SPDX-License-Identifier: AGPL-3.0-only
use std::ffi::{CStr, c_char};
use std::io::Read;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;

use tokio::sync::{mpsc as tokio_mpsc, watch};
use zeroize::Zeroizing;

const BACKEND_THREAD_STACK_BYTES: usize = 8 * 1024 * 1024;

use crate::backend::{self, Command, Config, EventNotification};
use crate::event::{ABI_VERSION, Event, OwnedEvent, SignalEvent};

const MAX_RECIPIENT_BYTES: usize = 256;
const MAX_MESSAGE_BYTES: usize = 64 * 1024;
const MAX_ATTACHMENT_BYTES: usize = 25 * 1024 * 1024;
const MAX_ATTACHMENT_FILENAME_BYTES: usize = 255;
const MAX_CONTENT_TYPE_BYTES: usize = 255;
const EVENT_QUEUE_CAPACITY: usize = 4096;

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
    shutdown: watch::Sender<bool>,
    events: Mutex<mpsc::Receiver<Event>>,
    event_notifier: UnixStream,
    event_notification: Arc<EventNotification>,
    event_overflowed: Arc<AtomicBool>,
    queued_event_bytes: Arc<AtomicUsize>,
    ready: Arc<AtomicBool>,
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
        unsafe { &*core }.event_notifier.as_raw_fd()
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
        let store_path = status_try!(unsafe { required_string(config.store_path, 4096) });
        // SAFETY: validated and copied by `required_string`.
        let device_name = status_try!(unsafe { required_string(config.device_name, 128) });
        // SAFETY: validated and copied by `required_string`.
        let passphrase = status_try!(unsafe { required_string(config.passphrase, 4096) });

        let (command_tx, command_rx) = tokio_mpsc::channel(128);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (event_tx, event_rx) = mpsc::sync_channel(EVENT_QUEUE_CAPACITY);
        let (event_notifier, event_notifier_writer) =
            status_try!(UnixStream::pair().map_err(|_| SignalStatus::InternalError));
        status_try!(
            event_notifier
                .set_nonblocking(true)
                .map_err(|_| SignalStatus::InternalError)
        );
        status_try!(
            event_notifier_writer
                .set_nonblocking(true)
                .map_err(|_| SignalStatus::InternalError)
        );
        let worker_event_notifier = Arc::new(event_notifier_writer);
        let event_notification = Arc::new(EventNotification::new());
        let worker_event_notification = Arc::clone(&event_notification);
        let event_overflowed = Arc::new(AtomicBool::new(false));
        let worker_event_overflowed = Arc::clone(&event_overflowed);
        let queued_event_bytes = Arc::new(AtomicUsize::new(0));
        let worker_queued_event_bytes = Arc::clone(&queued_event_bytes);
        let ready = Arc::new(AtomicBool::new(false));
        let worker_ready = Arc::clone(&ready);
        let worker_config = Config {
            store_path,
            device_name,
            passphrase: Zeroizing::new(passphrase),
        };

        let join = status_try!(
            std::thread::Builder::new()
                .name("signal-purple-core".into())
                .stack_size(BACKEND_THREAD_STACK_BYTES)
                .spawn(move || {
                    backend::run_worker(
                        worker_config,
                        command_rx,
                        shutdown_rx,
                        event_tx,
                        worker_event_notification,
                        worker_event_notifier,
                        worker_event_overflowed,
                        worker_queued_event_bytes,
                        worker_ready,
                    );
                })
                .map_err(|_| SignalStatus::InternalError)
        );

        let core = Box::new(SignalCore {
            commands: command_tx,
            shutdown: shutdown_tx,
            events: Mutex::new(event_rx),
            event_notifier,
            event_notification,
            event_overflowed,
            queued_event_bytes,
            ready,
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
        let group_key = status_try!(unsafe { required_string(group_key, 64) });
        if group_key.len() != 64 || hex::decode(&group_key).map_or(true, |v| v.len() != 32) {
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
        let group_key = status_try!(unsafe { required_string(group_key, 64) });
        if group_key.len() != 64 || hex::decode(&group_key).map_or(true, |v| v.len() != 32) {
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
        // SAFETY: all strings and bytes are copied during this call.
        let recipient =
            status_try!(unsafe { required_string(input.recipient, MAX_RECIPIENT_BYTES) });
        let filename =
            status_try!(unsafe { required_string(input.filename, MAX_ATTACHMENT_FILENAME_BYTES) });
        let content_type =
            status_try!(unsafe { required_string(input.content_type, MAX_CONTENT_TYPE_BYTES) });
        if group
            && (recipient.len() != 64
                || hex::decode(&recipient).map_or(true, |value| value.len() != 32))
        {
            return SignalStatus::InvalidArgument;
        }
        // SAFETY: the caller guarantees `data_len` readable bytes; the bound
        // above prevents an oversized allocation, and the bytes are copied.
        let data = unsafe { std::slice::from_raw_parts(input.data, input.data_len) }.to_vec();
        // SAFETY: `core` remains live and ABI calls are serialized by C.
        queue_command(
            unsafe { &*core },
            Command::SendAttachment {
                request_id,
                recipient,
                filename,
                content_type,
                data,
                group,
            },
        )
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
        queue_control_command(unsafe { &*core }, Command::CancelAttachment { request_id })
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
        queue_control_command(
            unsafe { &*core },
            Command::AcknowledgeMessage { delivery_id },
        )
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
        let _notification_guard = core.event_notification.lock();
        if core.event_overflowed.swap(false, Ordering::AcqRel) {
            let overflow = Event::error(
                "Signal event queue overflowed; reconnect to resynchronize messages",
                true,
            );
            // SAFETY: checked above; event ownership transfers to C.
            unsafe { *out_event = OwnedEvent::into_raw(overflow) };
            return 1;
        }
        let Ok(events) = core.events.lock() else {
            return -1;
        };
        match events.try_recv() {
            Ok(event) => {
                if !event.data.is_empty() {
                    core.queued_event_bytes
                        .fetch_sub(event.data.len(), Ordering::AcqRel);
                }
                // SAFETY: checked above; event ownership transfers to C.
                unsafe { *out_event = OwnedEvent::into_raw(event) };
                1
            }
            Err(error) => {
                let mut token = [0u8; 1];
                let mut notifier = &core.event_notifier;
                let _ = notifier.read(&mut token);
                core.event_notification.clear_pending();
                match error {
                    mpsc::TryRecvError::Empty => 0,
                    mpsc::TryRecvError::Disconnected => -1,
                }
            }
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
    use std::io::Write;

    fn test_core(
        commands: tokio_mpsc::Sender<Command>,
        shutdown: watch::Sender<bool>,
        join: Option<JoinHandle<()>>,
        ready: bool,
    ) -> SignalCore {
        let (_event_sender, events) = mpsc::sync_channel(1);
        let (event_notifier, _event_notifier_writer) = UnixStream::pair().unwrap();
        event_notifier.set_nonblocking(true).unwrap();
        SignalCore {
            commands,
            shutdown,
            events: Mutex::new(events),
            event_notifier,
            event_notification: Arc::new(EventNotification::new()),
            event_overflowed: Arc::new(AtomicBool::new(false)),
            queued_event_bytes: Arc::new(AtomicUsize::new(0)),
            ready: Arc::new(AtomicBool::new(ready)),
            join: Mutex::new(join),
        }
    }

    fn event_test_core(capacity: usize) -> (SignalCore, mpsc::SyncSender<Event>, UnixStream) {
        let (commands, _command_receiver) = tokio_mpsc::channel(1);
        let (shutdown, _shutdown_receiver) = watch::channel(false);
        let (event_sender, events) = mpsc::sync_channel(capacity);
        let (event_notifier, event_notifier_writer) = UnixStream::pair().unwrap();
        event_notifier.set_nonblocking(true).unwrap();
        event_notifier_writer.set_nonblocking(true).unwrap();
        let event_notification = Arc::new(EventNotification::new());
        (
            SignalCore {
                commands,
                shutdown,
                events: Mutex::new(events),
                event_notifier,
                event_notification,
                event_overflowed: Arc::new(AtomicBool::new(false)),
                queued_event_bytes: Arc::new(AtomicUsize::new(0)),
                ready: Arc::new(AtomicBool::new(true)),
                join: Mutex::new(None),
            },
            event_sender,
            event_notifier_writer,
        )
    }

    fn enqueue_test_event(
        core: &SignalCore,
        sender: &mpsc::SyncSender<Event>,
        writer: &mut UnixStream,
        event: Event,
    ) {
        let _guard = core.event_notification.lock();
        sender.try_send(event).unwrap();
        if !core.event_notification.mark_pending() {
            writer.write_all(&[1]).unwrap();
        }
    }

    fn assert_notification_readable(
        core: &SignalCore,
        writer: &mut UnixStream,
        token: &mut [u8; 1],
    ) {
        let _guard = core.event_notification.lock();
        assert_eq!((&core.event_notifier).read(token).unwrap(), 1);
        assert_eq!(*token, [1]);
        writer.write_all(&[1]).unwrap();
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
        let (core, _sender, _writer) = event_test_core(1);

        // SAFETY: `core` remains live for the duration of the call.
        assert_eq!(
            unsafe { signal_core_event_fd(&core) },
            core.event_notifier.as_raw_fd()
        );
        // SAFETY: a null core is an explicitly supported error path.
        assert_eq!(unsafe { signal_core_event_fd(std::ptr::null()) }, -1);
    }

    #[test]
    fn exact_event_batch_keeps_one_level_trigger_until_empty() {
        const BATCH_SIZE: usize = 64;
        let (mut core, sender, mut writer) = event_test_core(BATCH_SIZE);

        for request_id in 1..=BATCH_SIZE as u64 {
            enqueue_test_event(
                &core,
                &sender,
                &mut writer,
                Event {
                    kind: crate::event::EVENT_MESSAGE,
                    request_id,
                    ..Event::default()
                },
            );
        }

        let mut token = [0u8; 1];
        assert_notification_readable(&core, &mut writer, &mut token);
        for request_id in 1..=BATCH_SIZE as u64 {
            let mut event = std::ptr::null_mut();
            // SAFETY: the core and output pointer remain live for the call.
            assert_eq!(unsafe { signal_core_poll_event(&mut core, &mut event) }, 1);
            assert_eq!(unsafe { (*event).request_id }, request_id);
            // SAFETY: this test uniquely owns the returned event.
            unsafe { signal_event_free(event) };
        }

        assert!(core.event_notification.is_pending());
        assert_notification_readable(&core, &mut writer, &mut token);
        let mut event = std::ptr::dangling_mut::<SignalEvent>();
        // SAFETY: the core and output pointer remain live for the call.
        assert_eq!(unsafe { signal_core_poll_event(&mut core, &mut event) }, 0);
        assert!(event.is_null());
        assert!(!core.event_notification.is_pending());
        assert_eq!(
            (&core.event_notifier).read(&mut token).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );

        enqueue_test_event(
            &core,
            &sender,
            &mut writer,
            Event {
                kind: crate::event::EVENT_MESSAGE,
                request_id: 65,
                ..Event::default()
            },
        );
        assert_eq!((&core.event_notifier).read(&mut token).unwrap(), 1);
    }

    #[test]
    fn overflow_event_does_not_consume_a_real_event_notification() {
        let (mut core, sender, mut writer) = event_test_core(1);
        enqueue_test_event(
            &core,
            &sender,
            &mut writer,
            Event {
                kind: crate::event::EVENT_MESSAGE,
                request_id: 7,
                ..Event::default()
            },
        );
        core.event_overflowed.store(true, Ordering::Release);

        let mut event = std::ptr::null_mut();
        // SAFETY: the core and output pointer remain live for each call.
        assert_eq!(unsafe { signal_core_poll_event(&mut core, &mut event) }, 1);
        assert_eq!(unsafe { (*event).kind }, crate::event::EVENT_ERROR);
        // SAFETY: this test uniquely owns the returned event.
        unsafe { signal_event_free(event) };
        assert!(core.event_notification.is_pending());
        let mut token = [0u8; 1];
        assert_notification_readable(&core, &mut writer, &mut token);

        // SAFETY: the core and output pointer remain live for each call.
        assert_eq!(unsafe { signal_core_poll_event(&mut core, &mut event) }, 1);
        assert_eq!(unsafe { (*event).request_id }, 7);
        // SAFETY: this test uniquely owns the returned event.
        unsafe { signal_event_free(event) };
        // SAFETY: the core and output pointer remain live for each call.
        assert_eq!(unsafe { signal_core_poll_event(&mut core, &mut event) }, 0);
        assert!(!core.event_notification.is_pending());
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
        let (commands, mut receiver) = tokio_mpsc::channel(1);
        let (shutdown, _shutdown_receiver) = watch::channel(false);
        let core = test_core(commands, shutdown, None, false);

        assert_eq!(
            queue_control_command(&core, Command::AcknowledgeMessage { delivery_id: 42 }),
            SignalStatus::Ok
        );
        assert!(matches!(
            receiver.try_recv(),
            Ok(Command::AcknowledgeMessage { delivery_id: 42 })
        ));
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
        let core = Box::into_raw(Box::new(test_core(commands, shutdown, Some(join), true)));

        // SAFETY: this test uniquely owns the core allocation until free.
        unsafe { signal_core_shutdown(core) };
        assert!(stopped.load(Ordering::Acquire));
        // SAFETY: shutdown is idempotent and this test uniquely owns `core`.
        unsafe { signal_core_free(core) };
    }
}
