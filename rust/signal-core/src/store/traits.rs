// SPDX-License-Identifier: AGPL-3.0-only
use presage::libsignal_service::content::Content;
use presage::store::Thread;
use presage_store_sqlite::{ClientOutboxKind, ClientOutboxMessage, SqliteStoreError};

use super::errors::StorageError;
use super::repository::StorageRepository;

/// The outbox and message-projection-ledger surface of `StorageRepository` that
/// `backend/outbox.rs` and `backend/projection.rs` drive. Generic code depends on
/// this trait instead of the concrete `StorageRepository` so it can be exercised
/// in tests against an in-memory fake, without a real SQLite database.
///
/// `due_outbox_messages`/`complete_outbox_message`/`defer_outbox_message` and
/// `mark_sent_message_projected` are already reached polymorphically (via
/// `retry_outbox`/`enqueue_and_send`/`mark_sent_message_projected_or_report`).
/// The remaining methods mirror ones still called directly on a concrete
/// `StorageRepository` (e.g. from `handle_command`'s identity/outbox-cleanup
/// arms, `load_unprojected_messages`, `project_content`) — kept here so those
/// call sites can move to the trait incrementally without reshaping it again.
#[allow(dead_code)]
pub(crate) trait StorageOps {
    async fn due_outbox_messages(
        &self,
        now_ms: u64,
    ) -> Result<Vec<ClientOutboxMessage>, SqliteStoreError>;

    async fn enqueue_outbox_message(
        &self,
        kind: ClientOutboxKind,
        recipient: &str,
        body: &str,
        timestamp: u64,
    ) -> Result<i64, SqliteStoreError>;

    async fn complete_outbox_message(&self, id: i64) -> Result<(), SqliteStoreError>;

    async fn defer_outbox_message(
        &self,
        id: i64,
        attempts: u32,
        retry_at_ms: u64,
    ) -> Result<(), SqliteStoreError>;

    async fn expedite_outbox_messages(&self, recipient: &str) -> Result<(), SqliteStoreError>;

    async fn unprojected_messages(&self) -> Result<Vec<Content>, SqliteStoreError>;

    async fn mark_message_projected(&self, content: &Content) -> Result<(), SqliteStoreError>;

    async fn sent_message_content(
        &self,
        thread: &Thread,
        timestamp: u64,
    ) -> Result<Option<Content>, SqliteStoreError>;

    async fn mark_sent_message_projected(
        &self,
        thread: &Thread,
        timestamp: u64,
    ) -> Result<(), StorageError>;
}

impl StorageOps for StorageRepository {
    async fn due_outbox_messages(
        &self,
        now_ms: u64,
    ) -> Result<Vec<ClientOutboxMessage>, SqliteStoreError> {
        StorageRepository::due_outbox_messages(self, now_ms).await
    }

    async fn enqueue_outbox_message(
        &self,
        kind: ClientOutboxKind,
        recipient: &str,
        body: &str,
        timestamp: u64,
    ) -> Result<i64, SqliteStoreError> {
        StorageRepository::enqueue_outbox_message(self, kind, recipient, body, timestamp).await
    }

    async fn complete_outbox_message(&self, id: i64) -> Result<(), SqliteStoreError> {
        StorageRepository::complete_outbox_message(self, id).await
    }

    async fn defer_outbox_message(
        &self,
        id: i64,
        attempts: u32,
        retry_at_ms: u64,
    ) -> Result<(), SqliteStoreError> {
        StorageRepository::defer_outbox_message(self, id, attempts, retry_at_ms).await
    }

    async fn expedite_outbox_messages(&self, recipient: &str) -> Result<(), SqliteStoreError> {
        StorageRepository::expedite_outbox_messages(self, recipient).await
    }

    async fn unprojected_messages(&self) -> Result<Vec<Content>, SqliteStoreError> {
        StorageRepository::unprojected_messages(self).await
    }

    async fn mark_message_projected(&self, content: &Content) -> Result<(), SqliteStoreError> {
        StorageRepository::mark_message_projected(self, content).await
    }

    async fn sent_message_content(
        &self,
        thread: &Thread,
        timestamp: u64,
    ) -> Result<Option<Content>, SqliteStoreError> {
        StorageRepository::sent_message_content(self, thread, timestamp).await
    }

    async fn mark_sent_message_projected(
        &self,
        thread: &Thread,
        timestamp: u64,
    ) -> Result<(), StorageError> {
        StorageRepository::mark_sent_message_projected(self, thread, timestamp).await
    }
}

#[cfg(test)]
#[allow(
    dead_code,
    reason = "shared test fixture; not every method is exercised by every test"
)]
pub(crate) mod fake {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct FakeState {
        next_id: i64,
        outbox: Vec<ClientOutboxMessage>,
        due_at: std::collections::HashMap<i64, u64>,
        sent: std::collections::HashMap<(Thread, u64), Content>,
        projected: std::collections::HashSet<(Thread, u64)>,
        /// When set, `mark_sent_message_projected` fails with this error the next
        /// `fail_projection_attempts` times before succeeding, to exercise retry.
        projection_failure: Option<(StorageErrorKind, u32)>,
    }

    #[derive(Clone, Copy)]
    pub(crate) enum StorageErrorKind {
        Transient,
        NotFound,
    }

    #[derive(Default)]
    pub(crate) struct FakeStorageRepository {
        state: Mutex<FakeState>,
    }

    impl FakeStorageRepository {
        pub(crate) fn new() -> Self {
            Self::default()
        }

        pub(crate) fn stage_sent_message(&self, thread: Thread, timestamp: u64, content: Content) {
            self.state
                .lock()
                .unwrap()
                .sent
                .insert((thread, timestamp), content);
        }

        pub(crate) fn is_projected(&self, thread: &Thread, timestamp: u64) -> bool {
            self.state
                .lock()
                .unwrap()
                .projected
                .contains(&(thread.clone(), timestamp))
        }

        /// Makes the next `attempts` calls to `mark_sent_message_projected` fail with
        /// `kind`, then succeed. Used to assert that a transient/not-found failure is
        /// retried rather than silently dropping the projection marker.
        pub(crate) fn fail_next_projection_attempts(&self, kind: StorageErrorKind, attempts: u32) {
            self.state.lock().unwrap().projection_failure = Some((kind, attempts));
        }
    }

    impl StorageOps for FakeStorageRepository {
        async fn due_outbox_messages(
            &self,
            now_ms: u64,
        ) -> Result<Vec<ClientOutboxMessage>, SqliteStoreError> {
            let state = self.state.lock().unwrap();
            Ok(state
                .outbox
                .iter()
                .filter(|message| {
                    state
                        .due_at
                        .get(&message.id)
                        .is_none_or(|due| *due <= now_ms)
                })
                .cloned()
                .collect())
        }

        async fn enqueue_outbox_message(
            &self,
            kind: ClientOutboxKind,
            recipient: &str,
            body: &str,
            timestamp: u64,
        ) -> Result<i64, SqliteStoreError> {
            let mut state = self.state.lock().unwrap();
            state.next_id += 1;
            let id = state.next_id;
            state.outbox.push(ClientOutboxMessage {
                id,
                kind,
                recipient: recipient.to_owned(),
                body: body.to_owned(),
                timestamp,
                attempts: 0,
            });
            Ok(id)
        }

        async fn complete_outbox_message(&self, id: i64) -> Result<(), SqliteStoreError> {
            let mut state = self.state.lock().unwrap();
            state.outbox.retain(|message| message.id != id);
            state.due_at.remove(&id);
            Ok(())
        }

        async fn defer_outbox_message(
            &self,
            id: i64,
            attempts: u32,
            retry_at_ms: u64,
        ) -> Result<(), SqliteStoreError> {
            let mut state = self.state.lock().unwrap();
            if let Some(message) = state.outbox.iter_mut().find(|message| message.id == id) {
                message.attempts = attempts;
            }
            state.due_at.insert(id, retry_at_ms);
            Ok(())
        }

        async fn expedite_outbox_messages(&self, recipient: &str) -> Result<(), SqliteStoreError> {
            let mut state = self.state.lock().unwrap();
            let ids: Vec<i64> = state
                .outbox
                .iter()
                .filter(|message| message.recipient == recipient)
                .map(|message| message.id)
                .collect();
            for id in ids {
                state.due_at.insert(id, 0);
            }
            Ok(())
        }

        async fn unprojected_messages(&self) -> Result<Vec<Content>, SqliteStoreError> {
            let state = self.state.lock().unwrap();
            Ok(state
                .sent
                .iter()
                .filter(|(key, _)| !state.projected.contains(key))
                .map(|(_, content)| content.clone())
                .collect())
        }

        async fn mark_message_projected(&self, _content: &Content) -> Result<(), SqliteStoreError> {
            Ok(())
        }

        async fn sent_message_content(
            &self,
            thread: &Thread,
            timestamp: u64,
        ) -> Result<Option<Content>, SqliteStoreError> {
            Ok(self
                .state
                .lock()
                .unwrap()
                .sent
                .get(&(thread.clone(), timestamp))
                .cloned())
        }

        async fn mark_sent_message_projected(
            &self,
            thread: &Thread,
            timestamp: u64,
        ) -> Result<(), StorageError> {
            {
                let mut state = self.state.lock().unwrap();
                if let Some((kind, remaining)) = state.projection_failure {
                    if remaining > 0 {
                        state.projection_failure = Some((kind, remaining - 1));
                        return Err(match kind {
                            StorageErrorKind::Transient => StorageError::store(
                                "fake transient failure",
                                SqliteStoreError::Db(sqlx::Error::PoolTimedOut),
                            ),
                            StorageErrorKind::NotFound => {
                                StorageError::NotFound("fake row not yet visible")
                            }
                        });
                    }
                    state.projection_failure = None;
                }
            }
            self.state
                .lock()
                .unwrap()
                .projected
                .insert((thread.clone(), timestamp));
            Ok(())
        }
    }
}
