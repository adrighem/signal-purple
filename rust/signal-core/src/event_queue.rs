// SPDX-License-Identifier: AGPL-3.0-only
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};

use crate::event::Event;

const MAX_QUEUED_EVENT_BYTES: usize = 64 * 1024 * 1024;

struct EventQueueState {
    notification_pending: AtomicBool,
    closed: AtomicBool,
    queued_bytes: AtomicUsize,
    waiting_producers: AtomicUsize,
    serial: Mutex<()>,
    space_available: Condvar,
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
    if capacity == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "event queue capacity must be positive",
        ));
    }
    let (sender, receiver) = mpsc::sync_channel(capacity);
    let (notification_reader, notification_writer) = UnixStream::pair()?;
    notification_reader.set_nonblocking(true)?;
    notification_writer.set_nonblocking(true)?;
    let state = Arc::new(EventQueueState {
        notification_pending: AtomicBool::new(false),
        closed: AtomicBool::new(false),
        queued_bytes: AtomicUsize::new(0),
        waiting_producers: AtomicUsize::new(0),
        serial: Mutex::new(()),
        space_available: Condvar::new(),
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
        let event_bytes = event.data.len();
        let mut event = event;
        let mut guard = self
            .state
            .serial
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        loop {
            if self.state.closed.load(Ordering::Acquire) {
                return;
            }
            let queued_bytes = self.state.queued_bytes.load(Ordering::Acquire);
            if queued_bytes != 0
                && queued_bytes
                    .checked_add(event_bytes)
                    .is_none_or(|total| total > self.state.max_queued_bytes)
            {
                self.state.waiting_producers.fetch_add(1, Ordering::AcqRel);
                guard = self
                    .state
                    .space_available
                    .wait(guard)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                self.state.waiting_producers.fetch_sub(1, Ordering::AcqRel);
                continue;
            }
            if event_bytes > 0 {
                self.state
                    .queued_bytes
                    .fetch_add(event_bytes, Ordering::AcqRel);
            }
            match self.sender.try_send(event) {
                Ok(()) => {
                    self.notify_locked();
                    return;
                }
                Err(mpsc::TrySendError::Full(returned)) => {
                    if event_bytes > 0 {
                        self.state
                            .queued_bytes
                            .fetch_sub(event_bytes, Ordering::AcqRel);
                    }
                    event = returned;
                    self.state.waiting_producers.fetch_add(1, Ordering::AcqRel);
                    guard = self
                        .state
                        .space_available
                        .wait(guard)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    self.state.waiting_producers.fetch_sub(1, Ordering::AcqRel);
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    if event_bytes > 0 {
                        self.state
                            .queued_bytes
                            .fetch_sub(event_bytes, Ordering::AcqRel);
                    }
                    return;
                }
            }
        }
    }
}

impl EventQueue {
    pub(crate) fn event_fd(&self) -> RawFd {
        self.notification_reader.as_raw_fd()
    }

    pub(crate) fn close(&self) {
        let _guard = self
            .state
            .serial
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.state.closed.store(true, Ordering::Release);
        self.state.space_available.notify_all();
    }

    #[cfg(test)]
    pub(crate) fn waiting_producers(&self) -> usize {
        self.state.waiting_producers.load(Ordering::Acquire)
    }

    pub(crate) fn poll(&self) -> EventPoll {
        let _notification_guard = self
            .state
            .serial
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
                self.state.space_available.notify_all();
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

impl Drop for EventQueue {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{EVENT_GROUP_MESSAGE, EVENT_MESSAGE};
    use std::net::Shutdown;
    use std::thread;
    use std::time::Duration;

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

    fn assert_producer_waiting(queue: &EventQueue) {
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while queue.waiting_producers() == 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "producer did not enter event-queue backpressure"
            );
            thread::yield_now();
        }
    }

    #[test]
    fn applies_count_backpressure_without_losing_events() {
        let (sink, queue) = queue(1, 16);

        sink.emit(Event {
            kind: EVENT_MESSAGE,
            data: vec![0; 4],
            ..Event::default()
        });
        let (started_tx, started_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let blocked_sink = sink.clone();
        let producer = thread::spawn(move || {
            started_tx.send(()).unwrap();
            blocked_sink.emit(Event {
                kind: EVENT_GROUP_MESSAGE,
                data: vec![0; 4],
                ..Event::default()
            });
            finished_tx.send(()).unwrap();
        });

        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_producer_waiting(&queue);
        assert!(matches!(
            finished_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        assert_notification_readable(&queue, &sink);
        assert_eq!(queue.state.queued_bytes.load(Ordering::Acquire), 4);
        assert_event(queue.poll(), EVENT_MESSAGE);
        finished_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_notification_readable(&queue, &sink);
        assert_event(queue.poll(), EVENT_GROUP_MESSAGE);
        producer.join().unwrap();
    }

    #[test]
    fn applies_byte_backpressure_without_losing_events() {
        let (sink, queue) = queue(4, 8);

        sink.emit(Event {
            kind: EVENT_MESSAGE,
            data: vec![0; 8],
            ..Event::default()
        });
        let (started_tx, started_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let producer = thread::spawn(move || {
            started_tx.send(()).unwrap();
            sink.emit(Event {
                kind: EVENT_GROUP_MESSAGE,
                data: vec![0],
                ..Event::default()
            });
            finished_tx.send(()).unwrap();
        });

        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_producer_waiting(&queue);
        assert!(matches!(
            finished_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        assert_eq!(assert_event(queue.poll(), EVENT_MESSAGE).data.len(), 8);
        finished_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(
            assert_event(queue.poll(), EVENT_GROUP_MESSAGE).data.len(),
            1
        );
        producer.join().unwrap();
    }

    #[test]
    fn admits_one_event_larger_than_the_byte_budget() {
        let (sink, queue) = queue(2, 8);

        sink.emit(Event {
            kind: EVENT_MESSAGE,
            data: vec![0; 9],
            ..Event::default()
        });
        let (started_tx, started_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let blocked_sink = sink.clone();
        let producer = thread::spawn(move || {
            started_tx.send(()).unwrap();
            blocked_sink.emit(Event {
                kind: EVENT_GROUP_MESSAGE,
                data: vec![0],
                ..Event::default()
            });
            finished_tx.send(()).unwrap();
        });

        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_producer_waiting(&queue);
        assert!(matches!(
            finished_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        assert_eq!(queue.state.queued_bytes.load(Ordering::Acquire), 9);
        assert_eq!(assert_event(queue.poll(), EVENT_MESSAGE).data.len(), 9);
        finished_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(
            assert_event(queue.poll(), EVENT_GROUP_MESSAGE).data.len(),
            1
        );
        assert!(matches!(queue.poll(), EventPoll::Empty));
        producer.join().unwrap();
    }

    #[test]
    fn close_unblocks_a_backpressured_producer() {
        let (sink, queue) = queue(1, 8);
        sink.emit(Event {
            kind: EVENT_MESSAGE,
            ..Event::default()
        });
        let blocked_sink = sink.clone();
        let (started_tx, started_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let producer = thread::spawn(move || {
            started_tx.send(()).unwrap();
            blocked_sink.emit(Event {
                kind: EVENT_GROUP_MESSAGE,
                ..Event::default()
            });
            finished_tx.send(()).unwrap();
        });

        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_producer_waiting(&queue);
        assert!(matches!(
            finished_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        queue.close();
        finished_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_event(queue.poll(), EVENT_MESSAGE);
        assert!(matches!(queue.poll(), EventPoll::Empty));
        producer.join().unwrap();
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
