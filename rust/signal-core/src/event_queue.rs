// SPDX-License-Identifier: AGPL-3.0-only
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use crate::event::Event;

const MAX_QUEUED_EVENT_BYTES: usize = 64 * 1024 * 1024;

struct EventQueueState {
    notification_pending: AtomicBool,
    overflowed: AtomicBool,
    queued_bytes: AtomicUsize,
    serial: Mutex<()>,
    max_queued_bytes: usize,
}

#[derive(Clone)]
pub(crate) struct EventSink {
    sender: mpsc::SyncSender<Event>,
    notification_writer: Arc<UnixStream>,
    state: Arc<EventQueueState>,
}

pub(crate) struct EventQueue {
    receiver: Mutex<mpsc::Receiver<Event>>,
    notification_reader: UnixStream,
    state: Arc<EventQueueState>,
}

pub(crate) enum EventPoll {
    Event(Event),
    Overflow,
    Empty,
    Disconnected,
}

pub(crate) fn event_queue(capacity: usize) -> io::Result<(EventSink, EventQueue)> {
    event_queue_with_byte_limit(capacity, MAX_QUEUED_EVENT_BYTES)
}

fn event_queue_with_byte_limit(
    capacity: usize,
    max_queued_bytes: usize,
) -> io::Result<(EventSink, EventQueue)> {
    let (sender, receiver) = mpsc::sync_channel(capacity);
    let (notification_reader, notification_writer) = UnixStream::pair()?;
    notification_reader.set_nonblocking(true)?;
    notification_writer.set_nonblocking(true)?;
    let state = Arc::new(EventQueueState {
        notification_pending: AtomicBool::new(false),
        overflowed: AtomicBool::new(false),
        queued_bytes: AtomicUsize::new(0),
        serial: Mutex::new(()),
        max_queued_bytes,
    });
    Ok((
        EventSink {
            sender,
            notification_writer: Arc::new(notification_writer),
            state: Arc::clone(&state),
        },
        EventQueue {
            receiver: Mutex::new(receiver),
            notification_reader,
            state,
        },
    ))
}

impl EventSink {
    fn notify_locked(&self) {
        if self.state.notification_pending.swap(true, Ordering::AcqRel) {
            return;
        }

        let mut writer = self.notification_writer.as_ref();
        loop {
            match writer.write(&[1]) {
                Ok(1) => return,
                Ok(_) => {
                    self.state
                        .notification_pending
                        .store(false, Ordering::Release);
                    return;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                // A full socket means its peer is already readable, so the
                // level-trigger invariant still holds.
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return,
                Err(_) => {
                    self.state
                        .notification_pending
                        .store(false, Ordering::Release);
                    return;
                }
            }
        }
    }

    pub(crate) fn emit(&self, event: Event) {
        if self.state.overflowed.load(Ordering::Acquire) {
            return;
        }
        let event_bytes = event.data.len();
        if event_bytes > 0
            && self
                .state
                .queued_bytes
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |queued| {
                    queued
                        .checked_add(event_bytes)
                        .filter(|total| *total <= self.state.max_queued_bytes)
                })
                .is_err()
        {
            let _notification_guard = self
                .state
                .serial
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.state.overflowed.store(true, Ordering::Release);
            self.notify_locked();
            return;
        }
        let _notification_guard = self
            .state
            .serial
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match self.sender.try_send(event) {
            Ok(()) => self.notify_locked(),
            Err(error) => {
                if event_bytes > 0 {
                    self.state
                        .queued_bytes
                        .fetch_sub(event_bytes, Ordering::AcqRel);
                }
                if matches!(error, mpsc::TrySendError::Full(_)) {
                    self.state.overflowed.store(true, Ordering::Release);
                    self.notify_locked();
                }
            }
        }
    }
}

impl EventQueue {
    pub(crate) fn event_fd(&self) -> RawFd {
        self.notification_reader.as_raw_fd()
    }

    pub(crate) fn poll(&self) -> EventPoll {
        let _notification_guard = self
            .state
            .serial
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.state.overflowed.swap(false, Ordering::AcqRel) {
            return EventPoll::Overflow;
        }
        let events = match self.receiver.lock() {
            Ok(events) => events,
            Err(_) => return EventPoll::Disconnected,
        };
        match events.try_recv() {
            Ok(event) => {
                if !event.data.is_empty() {
                    self.state
                        .queued_bytes
                        .fetch_sub(event.data.len(), Ordering::AcqRel);
                }
                EventPoll::Event(event)
            }
            Err(error) => {
                let mut token = [0u8; 1];
                let mut reader = &self.notification_reader;
                let _ = reader.read(&mut token);
                self.state
                    .notification_pending
                    .store(false, Ordering::Release);
                match error {
                    mpsc::TryRecvError::Empty => EventPoll::Empty,
                    mpsc::TryRecvError::Disconnected => EventPoll::Disconnected,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{EVENT_GROUP_MESSAGE, EVENT_MESSAGE};
    use std::net::Shutdown;

    fn queue(capacity: usize, max_queued_bytes: usize) -> (EventSink, EventQueue) {
        event_queue_with_byte_limit(capacity, max_queued_bytes).unwrap()
    }

    fn assert_event(poll: EventPoll, kind: u32) -> Event {
        let EventPoll::Event(event) = poll else {
            panic!("expected an event");
        };
        assert_eq!(event.kind, kind);
        event
    }

    fn assert_notification_readable(queue: &EventQueue, sink: &EventSink) {
        let mut token = [0u8; 1];
        let mut reader = &queue.notification_reader;
        assert_eq!(reader.read(&mut token).unwrap(), 1);
        assert_eq!(token, [1]);
        assert_eq!(
            reader.read(&mut token).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        let mut writer = sink.notification_writer.as_ref();
        writer.write_all(&[1]).unwrap();
    }

    fn assert_notification_empty(queue: &EventQueue) {
        let mut token = [0u8; 1];
        let mut reader = &queue.notification_reader;
        assert_eq!(
            reader.read(&mut token).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn reports_count_overflow_without_growing_the_queue() {
        let (sink, queue) = queue(1, 16);

        sink.emit(Event {
            kind: EVENT_MESSAGE,
            data: vec![0; 4],
            ..Event::default()
        });
        sink.emit(Event {
            kind: EVENT_GROUP_MESSAGE,
            data: vec![0; 4],
            ..Event::default()
        });
        sink.emit(Event::default());

        assert_notification_readable(&queue, &sink);
        assert!(matches!(queue.poll(), EventPoll::Overflow));
        assert_notification_readable(&queue, &sink);
        assert_eq!(queue.state.queued_bytes.load(Ordering::Acquire), 4);
        assert_event(queue.poll(), EVENT_MESSAGE);
        assert_notification_readable(&queue, &sink);
        assert!(matches!(queue.poll(), EventPoll::Empty));
        assert_notification_empty(&queue);
        assert_eq!(queue.state.queued_bytes.load(Ordering::Acquire), 0);

        sink.emit(Event {
            kind: EVENT_MESSAGE,
            data: vec![0; 4],
            ..Event::default()
        });
        assert_notification_readable(&queue, &sink);
        assert_event(queue.poll(), EVENT_MESSAGE);
    }

    #[test]
    fn bounds_binary_events_with_a_small_test_budget() {
        let (sink, queue) = queue(4, 8);

        sink.emit(Event {
            kind: EVENT_MESSAGE,
            data: vec![0; 8],
            ..Event::default()
        });
        sink.emit(Event {
            kind: EVENT_MESSAGE,
            data: vec![0],
            ..Event::default()
        });

        assert!(matches!(queue.poll(), EventPoll::Overflow));
        assert_eq!(assert_event(queue.poll(), EVENT_MESSAGE).data.len(), 8);
        assert!(matches!(queue.poll(), EventPoll::Empty));
        assert_eq!(queue.state.queued_bytes.load(Ordering::Acquire), 0);
    }

    #[test]
    fn notifies_when_binary_overflow_happens_before_any_enqueue() {
        let (sink, queue) = queue(1, 8);

        sink.emit(Event {
            kind: EVENT_MESSAGE,
            data: vec![0; 9],
            ..Event::default()
        });

        assert!(matches!(queue.poll(), EventPoll::Overflow));
        assert!(matches!(queue.poll(), EventPoll::Empty));
    }

    #[test]
    fn failed_notification_write_does_not_leave_pending_set() {
        let (sink, queue) = queue(1, 8);
        queue.notification_reader.shutdown(Shutdown::Both).unwrap();

        sink.emit(Event {
            kind: EVENT_MESSAGE,
            ..Event::default()
        });

        assert!(!sink.state.notification_pending.load(Ordering::Acquire));
        assert_event(queue.poll(), EVENT_MESSAGE);
    }

    #[test]
    fn coalesces_notifications_until_the_queue_is_observed_empty() {
        let (sink, queue) = queue(2, 8);

        sink.emit(Event {
            kind: EVENT_MESSAGE,
            ..Event::default()
        });
        assert_notification_readable(&queue, &sink);
        assert_event(queue.poll(), EVENT_MESSAGE);
        assert_notification_readable(&queue, &sink);
        sink.emit(Event {
            kind: EVENT_GROUP_MESSAGE,
            ..Event::default()
        });

        assert!(sink.state.notification_pending.load(Ordering::Acquire));
        assert_notification_readable(&queue, &sink);
        assert_event(queue.poll(), EVENT_GROUP_MESSAGE);
        assert_notification_readable(&queue, &sink);
        assert!(matches!(queue.poll(), EventPoll::Empty));
        assert!(!sink.state.notification_pending.load(Ordering::Acquire));
        assert_notification_empty(&queue);

        sink.emit(Event {
            kind: EVENT_MESSAGE,
            ..Event::default()
        });
        assert!(sink.state.notification_pending.load(Ordering::Acquire));
        assert_notification_readable(&queue, &sink);
    }
}
