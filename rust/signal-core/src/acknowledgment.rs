// SPDX-License-Identifier: AGPL-3.0-only
use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

#[derive(Debug)]
struct AcknowledgmentState {
    open: bool,
    registered: HashSet<u64>,
    accepted: HashSet<u64>,
    ready: VecDeque<u64>,
    retry: VecDeque<u64>,
}

#[derive(Debug)]
pub(crate) struct AcknowledgmentInbox {
    state: Mutex<AcknowledgmentState>,
    changed: Notify,
}

impl AcknowledgmentInbox {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(AcknowledgmentState {
                open: true,
                registered: HashSet::new(),
                accepted: HashSet::new(),
                ready: VecDeque::new(),
                retry: VecDeque::new(),
            }),
            changed: Notify::new(),
        })
    }

    pub(crate) fn register(&self, delivery_id: u64) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .registered
            .insert(delivery_id);
    }

    pub(crate) fn unregister(&self, delivery_id: u64) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.registered.remove(&delivery_id);
        state.accepted.remove(&delivery_id);
        state.ready.retain(|queued| *queued != delivery_id);
        state.retry.retain(|queued| *queued != delivery_id);
    }

    pub(crate) fn submit(&self, delivery_id: u64) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if !state.open {
            return false;
        }
        let notify = state.registered.contains(&delivery_id) && state.accepted.insert(delivery_id);
        if notify {
            state.ready.push_back(delivery_id);
        }
        drop(state);
        if notify {
            self.changed.notify_one();
        }
        true
    }

    pub(crate) fn take_ready(&self, maximum: usize) -> Vec<u64> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let count = maximum.min(state.ready.len());
        let delivery_ids = state.ready.drain(..count).collect::<Vec<_>>();
        let has_more = !state.ready.is_empty();
        drop(state);
        if has_more {
            self.changed.notify_one();
        }
        delivery_ids
    }

    pub(crate) fn defer_retry(&self, delivery_id: u64) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.registered.contains(&delivery_id) && state.accepted.contains(&delivery_id) {
            state.retry.push_back(delivery_id);
        }
    }

    pub(crate) fn activate_retries(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let retries = state.retry.drain(..).collect::<Vec<_>>();
        state.ready.extend(retries);
        let notify = !state.ready.is_empty();
        drop(state);
        if notify {
            self.changed.notify_one();
        }
    }

    pub(crate) async fn wait(&self) {
        self.changed.notified().await;
    }

    pub(crate) fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.open = false;
        let retries = state.retry.drain(..).collect::<Vec<_>>();
        state.ready.extend(retries);
        drop(state);
        self.changed.notify_one();
    }

    #[cfg(test)]
    pub(crate) fn pending_len(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .accepted
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn coalesces_registered_ids_beyond_work_capacity() {
        let acknowledgments = AcknowledgmentInbox::new();

        for delivery_id in 1..=257 {
            acknowledgments.register(delivery_id);
            assert!(acknowledgments.submit(delivery_id));
            assert!(acknowledgments.submit(delivery_id));
        }
        assert!(acknowledgments.submit(999));
        assert_eq!(acknowledgments.pending_len(), 257);

        let mut drained = Vec::new();
        loop {
            let batch = acknowledgments.take_ready(64);
            if batch.is_empty() {
                break;
            }
            drained.extend(batch);
        }
        assert_eq!(drained, (1..=257).collect::<Vec<_>>());

        acknowledgments.close();
        assert!(!acknowledgments.submit(1));
    }

    #[test]
    fn retry_waits_for_explicit_reactivation() {
        let acknowledgments = AcknowledgmentInbox::new();
        acknowledgments.register(41);
        assert!(acknowledgments.submit(41));
        assert_eq!(acknowledgments.take_ready(1), [41]);

        acknowledgments.defer_retry(41);
        assert!(acknowledgments.take_ready(1).is_empty());
        acknowledgments.activate_retries();
        assert_eq!(acknowledgments.take_ready(1), [41]);

        acknowledgments.unregister(41);
        assert_eq!(acknowledgments.pending_len(), 0);
    }

    #[test]
    fn notification_does_not_remove_work_before_the_consumer_wins() {
        let acknowledgments = AcknowledgmentInbox::new();
        acknowledgments.register(41);
        assert!(acknowledgments.submit(41));

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_millis(50), acknowledgments.wait())
                .await
                .unwrap();
        });

        assert_eq!(acknowledgments.take_ready(1), [41]);
    }
}
