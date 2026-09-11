// SPDX-License-Identifier: AGPL-3.0-only
use presage::libsignal_service::content::Content;
use presage::libsignal_service::prelude::Uuid;
use presage::libsignal_service::protocol::{Aci, ServiceId};
use presage::libsignal_service::zkgroup::profiles::ProfileKey;
use presage::model::contacts::Contact;
use presage::model::groups::Group;
use presage::store::{ContentsStore, Thread};
use presage_store_sqlite::{
    ClientOutboxKind, ClientOutboxMessage, IdentityChangeNotice, SqliteStore, SqliteStoreError,
};

use super::errors::StorageError;

pub const MESSAGE_PROJECTION_CLIENT: &str = "signal-purple-v1";

/// Dedicated repository encapsulating all direct SQLite database interactions,
/// projections, outbox staging, and identity queries for `signal-core`.
#[derive(Clone)]
pub struct StorageRepository {
    store: SqliteStore,
}

impl StorageRepository {
    pub fn new(store: SqliteStore) -> Self {
        Self { store }
    }

    // --- Subsystem Initialization ---

    pub async fn initialize_subsystems(&self) -> Result<(), StorageError> {
        self.store
            .initialize_message_projection(MESSAGE_PROJECTION_CLIENT)
            .await
            .map_err(|source| {
                StorageError::store("Could not initialize durable message replay", source)
            })?;

        self.store
            .initialize_identity_change_tracking()
            .await
            .map_err(|source| {
                StorageError::store("Could not initialize identity-change tracking", source)
            })?;

        self.store
            .initialize_client_outbox()
            .await
            .map_err(|source| {
                StorageError::store("Could not initialize the encrypted outbox", source)
            })?;

        Ok(())
    }

    // --- Message Projection & Replay Queries ---

    pub async fn unprojected_messages(&self) -> Result<Vec<Content>, SqliteStoreError> {
        self.store
            .unprojected_messages(MESSAGE_PROJECTION_CLIENT)
            .await
    }

    pub async fn mark_message_projected(&self, content: &Content) -> Result<(), SqliteStoreError> {
        self.store
            .mark_message_projected(MESSAGE_PROJECTION_CLIENT, content)
            .await
    }

    pub async fn sent_message_content(
        &self,
        thread: &Thread,
        timestamp: u64,
    ) -> Result<Option<Content>, SqliteStoreError> {
        self.store.message(thread, timestamp).await
    }

    pub async fn mark_sent_message_projected(
        &self,
        thread: &Thread,
        timestamp: u64,
    ) -> Result<(), StorageError> {
        let content = self
            .sent_message_content(thread, timestamp)
            .await
            .map_err(|source| {
                StorageError::store("Could not read the sent Signal message", source)
            })?
            .ok_or(StorageError::NotFound(
                "The sent Signal message was not found in the encrypted store",
            ))?;

        self.mark_message_projected(&content)
            .await
            .map_err(|source| {
                StorageError::store("Could not record the sent Signal message", source)
            })
    }

    // --- Outbox Staging & Retries ---

    pub async fn due_outbox_messages(
        &self,
        now_ms: u64,
    ) -> Result<Vec<ClientOutboxMessage>, SqliteStoreError> {
        self.store.due_client_messages(now_ms).await
    }

    pub async fn enqueue_outbox_message(
        &self,
        kind: ClientOutboxKind,
        recipient: &str,
        body: &str,
        timestamp: u64,
    ) -> Result<i64, SqliteStoreError> {
        self.store
            .enqueue_client_message(kind, recipient, body, timestamp)
            .await
    }

    pub async fn complete_outbox_message(&self, id: i64) -> Result<(), SqliteStoreError> {
        self.store.complete_client_message(id).await
    }

    pub async fn defer_outbox_message(
        &self,
        id: i64,
        attempts: u32,
        retry_at_ms: u64,
    ) -> Result<(), SqliteStoreError> {
        self.store
            .defer_client_message(id, attempts, retry_at_ms)
            .await
    }

    pub async fn expedite_outbox_messages(&self, recipient: &str) -> Result<(), SqliteStoreError> {
        self.store.expedite_client_messages(recipient).await
    }

    // --- Identity Changes ---

    pub async fn identity_change_notices(
        &self,
    ) -> Result<Vec<IdentityChangeNotice>, SqliteStoreError> {
        self.store.identity_change_notices().await
    }

    pub async fn accept_identity_change(&self, recipient: &str) -> Result<bool, SqliteStoreError> {
        self.store.accept_identity_change(recipient).await
    }

    pub async fn dismiss_identity_change(&self, recipient: &str) -> Result<(), SqliteStoreError> {
        self.store.dismiss_identity_change(recipient).await
    }

    // --- Contacts & Avatars ---

    pub async fn contacts(&self) -> Result<Vec<Contact>, StorageError> {
        let stream = self.store.contacts().await.map_err(|source| {
            StorageError::store("Could not read synchronized Signal contacts", source)
        })?;

        stream.collect::<Result<Vec<_>, _>>().map_err(|source| {
            StorageError::store("Could not decode synchronized Signal contacts", source)
        })
    }

    pub async fn contact_profile_key(
        &self,
        service_id: &ServiceId,
    ) -> Result<Option<ProfileKey>, SqliteStoreError> {
        self.store.profile_key(service_id).await
    }

    pub async fn contact_avatar(
        &self,
        uuid: Uuid,
        profile_key: ProfileKey,
    ) -> Result<Option<Vec<u8>>, SqliteStoreError> {
        self.store.profile_avatar(uuid, profile_key).await
    }

    // --- Groups & Avatars ---

    pub async fn groups(&self) -> Result<Vec<([u8; 32], Group)>, StorageError> {
        let stream = self.store.groups().await.map_err(|source| {
            StorageError::store("Could not read synchronized Signal groups", source)
        })?;

        stream.collect::<Result<Vec<_>, _>>().map_err(|source| {
            StorageError::store("Could not decode synchronized Signal groups", source)
        })
    }

    pub async fn group(&self, key: [u8; 32]) -> Result<Option<Group>, SqliteStoreError> {
        self.store.group(key).await
    }

    pub async fn group_avatar(&self, key: [u8; 32]) -> Result<Option<Vec<u8>>, SqliteStoreError> {
        self.store.group_avatar(key).await
    }

    pub async fn active_group(
        &self,
        key: [u8; 32],
        local_aci: &Aci,
    ) -> Result<Option<Group>, StorageError> {
        self.group(key)
            .await
            .map(|group| group.filter(|g| group_has_local_aci(g, local_aci)))
            .map_err(|source| StorageError::store("Could not read Signal group membership", source))
    }
}

fn group_has_local_aci(group: &Group, local_aci: &Aci) -> bool {
    group.members.iter().any(|member| &member.aci == local_aci)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_constants_are_valid() {
        assert_eq!(MESSAGE_PROJECTION_CLIENT, "signal-purple-v1");
    }
}
