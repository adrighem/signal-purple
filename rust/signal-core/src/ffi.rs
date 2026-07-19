// SPDX-License-Identifier: AGPL-3.0-only
use std::ffi::{CStr, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;

use tokio::sync::{mpsc as tokio_mpsc, watch};
use zeroize::Zeroizing;

const BACKEND_THREAD_STACK_BYTES: usize = 8 * 1024 * 1024;

use crate::backend::{self, Command, Config};
use crate::event::{ABI_VERSION, Event, OwnedEvent, SignalEvent};

const MAX_RECIPIENT_BYTES: usize = 256;
const MAX_MESSAGE_BYTES: usize = 64 * 1024;
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
    event_overflowed: Arc<AtomicBool>,
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

#[unsafe(no_mangle)]
pub extern "C" fn signal_core_abi_version() -> u32 {
    ABI_VERSION
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
        let event_overflowed = Arc::new(AtomicBool::new(false));
        let worker_event_overflowed = Arc::clone(&event_overflowed);
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
                        worker_event_overflowed,
                        worker_ready,
                    );
                })
                .map_err(|_| SignalStatus::InternalError)
        );

        let core = Box::new(SignalCore {
            commands: command_tx,
            shutdown: shutdown_tx,
            events: Mutex::new(event_rx),
            event_overflowed,
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
                // SAFETY: checked above; event ownership transfers to C.
                unsafe { *out_event = OwnedEvent::into_raw(event) };
                1
            }
            Err(mpsc::TryRecvError::Empty) => 0,
            Err(mpsc::TryRecvError::Disconnected) => -1,
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

    fn test_core(
        commands: tokio_mpsc::Sender<Command>,
        shutdown: watch::Sender<bool>,
        join: Option<JoinHandle<()>>,
        ready: bool,
    ) -> SignalCore {
        let (_event_sender, events) = mpsc::sync_channel(1);
        SignalCore {
            commands,
            shutdown,
            events: Mutex::new(events),
            event_overflowed: Arc::new(AtomicBool::new(false)),
            ready: Arc::new(AtomicBool::new(ready)),
            join: Mutex::new(join),
        }
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
