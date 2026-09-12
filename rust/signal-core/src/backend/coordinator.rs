// SPDX-License-Identifier: AGPL-3.0-only
use std::collections::{HashMap, HashSet};
#[cfg(test)]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::channel::oneshot;
use presage::libsignal_service::content::{
    Content, ContentBody, DataMessage, GroupContextV2, ServiceError,
};
use presage::libsignal_service::groups_v2::Role;
#[cfg(test)]
use presage::libsignal_service::protocol::SignalProtocolError;
use presage::libsignal_service::protocol::{Aci, ServiceId};
use presage::libsignal_service::sender::{AttachmentSpec, MessageSenderError};
use presage::libsignal_service::zkgroup::profiles::ProfileKey;
use presage::model::groups::Group;
use presage::proto::{
    AttachmentPointer, EditMessage, ReceiptMessage, SyncMessage, TypingMessage, receipt_message,
    typing_message,
};
use presage::store::Thread;
use presage::{Manager, manager::Registered};
use presage_store_sqlite::ClientOutboxKind;
use presage_store_sqlite::SqliteStore;
use qrcode::QrCode;
use qrcode::types::Color;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex as AsyncMutex, mpsc as tokio_mpsc, watch};

use super::command::Command;
use super::media::{
    AvatarCache, DownloadedAttachment, MAX_SIGNAL_GIF_TRANSCODES_PER_MESSAGE,
    attachment_display_name, should_inline_image, transcode_signal_gif_video,
};
use super::outbox::{NewOutboxMessage, enqueue_and_send, retry_outbox};
use super::projection::*;
use super::protocol::SignalProtocol;
use super::shutdown::{run_after_start_signal, wait_for_shutdown};
use crate::attachment::{
    AttachmentControl, AttachmentPayload, AttachmentPermit, MAX_ATTACHMENT_BYTES,
};
use crate::event::{
    EVENT_ACCOUNT, EVENT_ATTACHMENT, EVENT_ATTACHMENT_SENT, EVENT_AVATAR, EVENT_CONTACT,
    EVENT_CONTACT_SYNC_BEGIN, EVENT_CONTACT_SYNC_END, EVENT_GROUP, EVENT_GROUP_LEFT,
    EVENT_GROUP_MEMBER, EVENT_GROUP_MESSAGE, EVENT_GROUP_SYNC_BEGIN, EVENT_GROUP_SYNC_END,
    EVENT_IDENTITY_ACCEPTED, EVENT_IDENTITY_CHANGE, EVENT_MESSAGE, EVENT_RECEIPT,
    EVENT_SESSION_RESET, EVENT_TYPING, Event, FLAG_OUTGOING,
};
use crate::event_queue::EventSink;
use crate::store::StorageRepository;
use crate::store::errors::{
    StorageError, signal_protocol_error_is_transient, sqlite_store_error_is_transient,
};
use crate::store::traits::StorageOps;

pub(crate) const GROUP_SYNC_RETRY_SECS: u64 = 30;
pub(crate) const RECOVERY_RETRY_DELAYS_SECS: [u64; 6] = [0, 1, 2, 4, 8, 16];
pub(crate) const RECEIVE_EVENT_QUEUE_CAPACITY: usize = 16;
pub(crate) const SHUTDOWN_CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
pub(crate) const SNAPSHOT_YIELD_INTERVAL: usize = 64;
/// Signal caps a single message at 32 attachments; anything beyond that in a
/// decoded message is malformed or hostile, not just unusually large, so the
/// rest are left undownloaded rather than serially downloading an unbounded
/// attacker-controlled count.
pub(crate) const MAX_ATTACHMENT_DOWNLOADS_PER_MESSAGE: usize = 32;

#[derive(Clone, Default)]
pub(crate) struct MessageTimestampAllocator {
    latest: Arc<AtomicU64>,
}

impl MessageTimestampAllocator {
    pub(crate) fn next(&self) -> u64 {
        self.next_at(wall_clock_ms())
    }

    fn next_at(&self, wall_clock_ms: u64) -> u64 {
        let mut previous = self.latest.load(Ordering::Relaxed);
        loop {
            let minimum = previous
                .checked_add(1)
                .expect("Signal message timestamp space was exhausted");
            let next = wall_clock_ms.max(minimum);
            match self.latest.compare_exchange_weak(
                previous,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return next,
                Err(observed) => previous = observed,
            }
        }
    }
}

/// Caches a Signal group's non-secret hashed identifier (see [`group_identifier`])
/// alongside its master key, so a lookup by identifier does not need a full,
/// every-group scan; and a contact's last known display name, used as a
/// fallback when a later sync briefly reports an empty name for that contact.
#[derive(Clone, Default)]
pub(crate) struct MetadataCache {
    group_index: Arc<Mutex<HashMap<String, [u8; 32]>>>,
    contact_names: Arc<Mutex<HashMap<String, String>>>,
}

impl MetadataCache {
    pub(crate) fn group_key_for_identifier(&self, identifier: &str) -> Option<[u8; 32]> {
        self.group_index.lock().ok()?.get(identifier).copied()
    }

    pub(crate) fn index_group(&self, identifier: String, master_key: [u8; 32]) {
        if let Ok(mut guard) = self.group_index.lock() {
            guard.insert(identifier, master_key);
        }
    }

    pub(crate) fn remove_group_index(&self, identifier: &str) {
        if let Ok(mut guard) = self.group_index.lock() {
            guard.remove(identifier);
        }
    }

    pub(crate) fn get_contact_name(&self, peer_id: &str) -> Option<String> {
        self.contact_names.lock().ok()?.get(peer_id).cloned()
    }

    pub(crate) fn put_contact_name(&self, peer_id: String, name: String) {
        if let Ok(mut guard) = self.contact_names.lock() {
            guard.insert(peer_id, name);
        }
    }

    #[cfg(test)]
    pub(crate) fn invalidate_contact(&self, peer_id: &str) {
        if let Ok(mut guard) = self.contact_names.lock() {
            guard.remove(peer_id);
        }
    }

    #[cfg(test)]
    pub(crate) fn clear(&self) {
        if let Ok(mut guard) = self.group_index.lock() {
            guard.clear();
        }
        if let Ok(mut guard) = self.contact_names.lock() {
            guard.clear();
        }
    }
}

#[derive(Default)]
pub(crate) struct RecoveryBackoff {
    next_delay: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum SessionPhase {
    #[default]
    Initializing,
    Recovering,
    Ready,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum GroupSnapshotState {
    #[default]
    Pending,
    Authoritative,
    Dirty,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RecoveryTransition {
    Entered,
    Continued,
}

#[derive(Default)]
pub(crate) struct SessionState {
    phase: SessionPhase,
    groups: GroupSnapshotState,
    recovery_backoff: RecoveryBackoff,
    last_recovery_error: Option<String>,
}

pub(crate) struct ReceiveStartError {
    pub(crate) message: String,
    pub(crate) transient: bool,
}

pub(crate) struct ActiveReceiveTasks {
    pub(crate) receive: tokio::task::JoinHandle<()>,
    pub(crate) contact_sync: tokio::task::JoinHandle<()>,
    pub(crate) avatar_fetch: tokio::task::JoinHandle<()>,
    pub(crate) group_sync: Option<tokio::task::JoinHandle<()>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProjectionDisposition {
    AwaitingAck,
    Complete,
    Retry,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProjectionEffect {
    pub(crate) remove_pending: bool,
    pub(crate) mark_projected: bool,
}

pub(crate) fn projection_effect(disposition: ProjectionDisposition) -> ProjectionEffect {
    match disposition {
        ProjectionDisposition::AwaitingAck => ProjectionEffect {
            remove_pending: false,
            mark_projected: false,
        },
        ProjectionDisposition::Complete => ProjectionEffect {
            remove_pending: true,
            mark_projected: true,
        },
        ProjectionDisposition::Retry => ProjectionEffect {
            remove_pending: true,
            mark_projected: false,
        },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GroupMessageTarget {
    Direct,
    Group([u8; 32]),
    Malformed,
}

#[derive(Debug, Eq, PartialEq)]
struct BareDataMessageRoute {
    peer: String,
    outgoing: bool,
}

pub(crate) struct SentMessage {
    pub(crate) thread: Thread,
    pub(crate) timestamp: u64,
}

enum ProjectionGroup {
    Active(Group),
    Complete,
    Retry,
}

fn group_message_target(message: &DataMessage) -> GroupMessageTarget {
    let Some(group) = message.group_v2.as_ref() else {
        return GroupMessageTarget::Direct;
    };
    match group
        .master_key
        .as_deref()
        .and_then(|key| <[u8; 32]>::try_from(key).ok())
    {
        Some(key) => GroupMessageTarget::Group(key),
        None => GroupMessageTarget::Malformed,
    }
}

fn bare_data_message_route(
    sender: ServiceId,
    destination: ServiceId,
    local_aci: Aci,
) -> BareDataMessageRoute {
    let outgoing = sender == ServiceId::Aci(local_aci);
    BareDataMessageRoute {
        peer: if outgoing { destination } else { sender }.service_id_string(),
        outgoing,
    }
}

fn group_message_peer(outgoing: bool, peer: &str, local_aci: Aci) -> String {
    if outgoing {
        ServiceId::Aci(local_aci).service_id_string()
    } else {
        peer.to_owned()
    }
}

#[derive(Clone, Default)]
pub(crate) struct DepartedGroups {
    state: Arc<Mutex<GroupLeaveState>>,
    operation: Arc<AsyncMutex<()>>,
}

#[derive(Default)]
struct GroupLeaveState {
    leaving: HashSet<String>,
    departed: HashSet<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GroupDepartureState {
    Active,
    Leaving,
    Departed,
}

fn departure_projection_disposition(state: GroupDepartureState) -> Option<ProjectionDisposition> {
    match state {
        GroupDepartureState::Active => None,
        GroupDepartureState::Leaving => Some(ProjectionDisposition::Retry),
        GroupDepartureState::Departed => Some(ProjectionDisposition::Complete),
    }
}

impl DepartedGroups {
    fn departure_state(&self, identifier: &str) -> GroupDepartureState {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.departed.contains(identifier) {
            GroupDepartureState::Departed
        } else if state.leaving.contains(identifier) {
            GroupDepartureState::Leaving
        } else {
            GroupDepartureState::Active
        }
    }

    pub(crate) fn contains(&self, identifier: &str) -> bool {
        self.departure_state(identifier) != GroupDepartureState::Active
    }

    fn is_departed(&self, identifier: &str) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .departed
            .contains(identifier)
    }

    pub(crate) fn begin_leave(&self, identifier: String) {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .leaving
            .insert(identifier);
    }

    fn cancel_leave(&self, identifier: &str) {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .leaving
            .remove(identifier);
    }

    fn mark_departed(&self, identifier: String) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.leaving.remove(&identifier);
        state.departed.insert(identifier);
    }

    pub(crate) async fn lock_operation(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.operation.lock().await
    }
}

enum GroupLeaveCompletion {
    Accepted {
        peer_notification_sent: bool,
        local_group_removed: bool,
    },
    Failed(String),
}

fn group_leave_completion_events(
    departed_groups: &DepartedGroups,
    request_id: u64,
    group_key: &str,
    completion: GroupLeaveCompletion,
) -> Vec<Event> {
    match completion {
        GroupLeaveCompletion::Accepted {
            peer_notification_sent,
            local_group_removed,
        } => {
            departed_groups.mark_departed(group_key.to_owned());
            let mut events = vec![Event {
                kind: EVENT_GROUP_LEFT,
                request_id,
                chat_id: Some(group_key.to_owned()),
                ..Event::default()
            }];
            events.extend(
                group_leave_warning_messages(peer_notification_sent, local_group_removed)
                    .into_iter()
                    .map(|warning| Event::error(warning, false)),
            );
            events
        }
        GroupLeaveCompletion::Failed(error) => {
            departed_groups.cancel_leave(group_key);
            vec![Event::group_request_error(request_id, group_key, error)]
        }
    }
}

impl RecoveryBackoff {
    /// Returns the next backoff delay. Once the fixed table is exhausted this
    /// keeps repeating its last (longest) entry indefinitely rather than
    /// signalling exhaustion: reconnection is retried for as long as the
    /// underlying error stays transient, since a network outage or a laptop
    /// suspend can easily outlast a fixed handful of retries, and giving up
    /// permanently there would force the user into a full manual reconnect
    /// for what is, from the account's perspective, just a slow network.
    fn next_delay(&mut self) -> Duration {
        let index = self.next_delay.min(RECOVERY_RETRY_DELAYS_SECS.len() - 1);
        self.next_delay = self.next_delay.saturating_add(1);
        Duration::from_secs(RECOVERY_RETRY_DELAYS_SECS[index])
    }

    fn reset(&mut self) {
        self.next_delay = 0;
    }
}

impl SessionState {
    pub(crate) fn is_recovering(&self) -> bool {
        self.phase == SessionPhase::Recovering
    }

    pub(crate) fn is_ready(&self) -> bool {
        self.phase == SessionPhase::Ready
    }

    pub(crate) fn groups_authoritative(&self) -> bool {
        matches!(
            self.groups,
            GroupSnapshotState::Authoritative | GroupSnapshotState::Dirty
        )
    }

    pub(crate) fn groups_dirty(&self) -> bool {
        self.groups == GroupSnapshotState::Dirty
    }

    pub(crate) fn note_group_content(&mut self, has_group_context: bool) {
        if has_group_context && self.groups == GroupSnapshotState::Authoritative {
            self.groups = GroupSnapshotState::Dirty;
        }
    }

    pub(crate) fn mark_groups_authoritative(&mut self) {
        self.groups = GroupSnapshotState::Authoritative;
    }

    pub(crate) fn mark_groups_pending(&mut self) {
        self.groups = GroupSnapshotState::Pending;
    }

    pub(crate) fn mark_ready(&mut self) {
        debug_assert!(!self.is_ready());
        self.phase = SessionPhase::Ready;
        self.recovery_backoff.reset();
    }

    pub(crate) fn enter_recovery(&mut self, error: String) -> RecoveryTransition {
        let transition = if self.is_recovering() {
            RecoveryTransition::Continued
        } else {
            RecoveryTransition::Entered
        };
        self.phase = SessionPhase::Recovering;
        self.groups = GroupSnapshotState::Pending;
        self.last_recovery_error = Some(error);
        transition
    }

    pub(crate) fn next_recovery_delay(&mut self) -> Duration {
        debug_assert!(self.is_recovering());
        self.recovery_backoff.next_delay()
    }

    #[cfg(test)]
    pub(crate) fn last_recovery_error(&self) -> Option<&str> {
        self.last_recovery_error.as_deref()
    }
}

fn retryable_http_status(status: u16) -> bool {
    matches!(status, 408 | 425 | 429) || (500..=599).contains(&status)
}

fn websocket_error_is_transient(error: &reqwest_websocket::Error) -> bool {
    match error {
        reqwest_websocket::Error::Handshake(
            reqwest_websocket::HandshakeError::UnexpectedStatusCode(status),
        ) => retryable_http_status(status.as_u16()),
        reqwest_websocket::Error::Handshake(_) => false,
        reqwest_websocket::Error::Reqwest(error) => {
            error.is_connect()
                || error.is_timeout()
                || error
                    .status()
                    .is_some_and(|status| retryable_http_status(status.as_u16()))
        }
        reqwest_websocket::Error::Tungstenite(_) => true,
        _ => false,
    }
}

fn service_error_is_transient(error: &ServiceError) -> bool {
    match error {
        ServiceError::Timeout { .. }
        | ServiceError::SendError { .. }
        | ServiceError::IO(_)
        | ServiceError::RateLimitExceeded { .. }
        | ServiceError::WsClosing { .. } => true,
        ServiceError::WsError(error) => websocket_error_is_transient(error),
        ServiceError::UnhandledResponseCode { status, .. } => {
            retryable_http_status(status.as_u16())
        }
        ServiceError::Http(error) => {
            error.is_connect()
                || error.is_timeout()
                || error
                    .status()
                    .is_some_and(|status| retryable_http_status(status.as_u16()))
        }
        ServiceError::SignalProtocolError(error) => signal_protocol_error_is_transient(error),
        _ => false,
    }
}

fn message_sender_error_is_transient(error: &MessageSenderError) -> bool {
    match error {
        MessageSenderError::ServiceError(error) => service_error_is_transient(error),
        MessageSenderError::ProtocolError(error) => signal_protocol_error_is_transient(error),
        _ => false,
    }
}

pub(crate) fn receive_error_is_transient(
    error: &presage::Error<presage_store_sqlite::SqliteStoreError>,
) -> bool {
    match error {
        presage::Error::IoError(_)
        | presage::Error::Timeout(_)
        | presage::Error::MessagePipeInterruptedError => true,
        presage::Error::ServiceError(error) => service_error_is_transient(error),
        presage::Error::MessageSenderError(error) => {
            message_sender_error_is_transient(error.as_ref())
        }
        presage::Error::ProtocolError(error) => signal_protocol_error_is_transient(error),
        presage::Error::Store(error) => sqlite_store_error_is_transient(error),
        _ => false,
    }
}

fn service_error_indicates_closed_websocket(error: &ServiceError) -> bool {
    matches!(error, ServiceError::WsClosing { .. })
        || matches!(error, ServiceError::WsError(error)
            if matches!(error.as_ref(), reqwest_websocket::Error::Tungstenite(_)))
}

fn receipt_error_indicates_closed_websocket(
    error: &presage::Error<presage_store_sqlite::SqliteStoreError>,
) -> bool {
    match error {
        presage::Error::MessagePipeInterruptedError => true,
        presage::Error::ServiceError(error) => service_error_indicates_closed_websocket(error),
        presage::Error::MessageSenderError(error) => {
            matches!(error.as_ref(), MessageSenderError::ServiceError(error)
                if service_error_indicates_closed_websocket(error))
        }
        _ => false,
    }
}

pub(crate) fn delivery_receipt_failure_action(
    error: &presage::Error<presage_store_sqlite::SqliteStoreError>,
) -> DeliveryReceiptFailureAction {
    if !receive_error_is_transient(error) {
        DeliveryReceiptFailureAction::Discard
    } else if receipt_error_indicates_closed_websocket(error) {
        DeliveryReceiptFailureAction::Recover
    } else {
        DeliveryReceiptFailureAction::Retry
    }
}

pub(crate) async fn request_contacts_after_queue_drain(
    start: oneshot::Receiver<()>,
    manager: Manager<SqliteStore, Registered>,
    shutdown: watch::Receiver<bool>,
    sink: EventSink,
) {
    run_after_start_signal(
        start,
        request_contacts_with_retries(manager, shutdown, sink),
    )
    .await;
}

async fn request_contacts_with_retries(
    mut manager: Manager<SqliteStore, Registered>,
    mut shutdown: watch::Receiver<bool>,
    sink: EventSink,
) {
    let mut backoff = RecoveryBackoff::default();

    loop {
        let result = {
            let mut request = Box::pin(manager.request_contacts());
            tokio::select! {
                result = &mut request => result,
                _ = wait_for_shutdown(&mut shutdown) => return,
            }
        };
        match result {
            Ok(()) => return,
            Err(error) => {
                let error = format!("Could not request Signal contact synchronization: {error}");
                let delay = backoff.next_delay();
                sink.emit(Event::transient_error(format!(
                    "{error}; retrying automatically"
                )));
                if !delay.is_zero() {
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = wait_for_shutdown(&mut shutdown) => return,
                    }
                }
            }
        }
    }
}

pub(crate) async fn fetch_missing_avatars_after_queue_drain(
    start: oneshot::Receiver<()>,
    manager: Manager<SqliteStore, Registered>,
    shutdown: watch::Receiver<bool>,
    sink: EventSink,
    avatar_cache: AvatarCache,
    metadata_cache: MetadataCache,
) {
    run_after_start_signal(
        start,
        fetch_missing_avatars(manager, shutdown, sink, avatar_cache, metadata_cache),
    )
    .await;
}

async fn fetch_missing_avatars(
    mut manager: Manager<SqliteStore, Registered>,
    mut shutdown: watch::Receiver<bool>,
    sink: EventSink,
    avatar_cache: AvatarCache,
    metadata_cache: MetadataCache,
) {
    let repo = StorageRepository::new(manager.store().clone());
    if let Ok(contacts) = repo.contacts().await {
        for contact in contacts {
            tokio::task::yield_now().await;
            let profile_key = if contact.profile_key.len() == 32 {
                let mut key_bytes = [0u8; 32];
                key_bytes.copy_from_slice(&contact.profile_key);
                Some(ProfileKey::create(key_bytes))
            } else {
                match repo
                    .contact_profile_key(&ServiceId::Aci(contact.uuid.into()))
                    .await
                {
                    Ok(key) => key,
                    Err(error) if sqlite_store_error_is_transient(&error) => {
                        tracing::warn!(
                            %error,
                            "Transient store contention reading a Signal contact profile key; skipping this cycle"
                        );
                        None
                    }
                    Err(error) => {
                        tracing::warn!(%error, "Could not read a Signal contact profile key");
                        None
                    }
                }
            };

            if let Some(key) = profile_key {
                let is_cached = match repo.contact_avatar(contact.uuid, key).await {
                    Ok(avatar) => avatar.is_some(),
                    Err(error) if sqlite_store_error_is_transient(&error) => {
                        tracing::warn!(
                            %error,
                            "Transient store contention reading a cached Signal contact avatar"
                        );
                        false
                    }
                    Err(error) => {
                        tracing::warn!(%error, "Could not read a cached Signal contact avatar");
                        false
                    }
                };
                if !is_cached {
                    let fetch = manager.retrieve_profile_avatar_by_uuid(contact.uuid, key);
                    let result = tokio::select! {
                        res = fetch => res,
                        _ = wait_for_shutdown(&mut shutdown) => return,
                    };
                    if let Ok(Some(avatar)) = result {
                        let (avatar_data, checksum) = avatar_cache.prepare_avatar(avatar);
                        let peer = ServiceId::Aci(contact.uuid.into()).service_id_string();
                        sink.emit(Event {
                            kind: EVENT_AVATAR,
                            peer_id: Some(peer),
                            title: Some(checksum),
                            data: avatar_data,
                            ..Event::default()
                        });
                    }
                }
            }
        }
    }

    if let Ok(groups) = repo.groups().await {
        let local_aci = manager.registration_data().service_ids.aci();
        for (key, group) in groups {
            tokio::task::yield_now().await;
            if !group_contains_local_aci(&group, &local_aci) || group.avatar.is_empty() {
                continue;
            }
            metadata_cache.index_group(group_identifier(&key), key);
            let is_cached = match repo.group_avatar(key).await {
                Ok(avatar) => avatar.is_some(),
                Err(error) if sqlite_store_error_is_transient(&error) => {
                    tracing::warn!(%error, "Transient store contention reading a cached Signal group avatar");
                    false
                }
                Err(error) => {
                    tracing::warn!(%error, "Could not read a cached Signal group avatar");
                    false
                }
            };
            if !is_cached {
                let context = GroupContextV2 {
                    master_key: Some(key.to_vec()),
                    revision: Some(group.revision),
                    ..Default::default()
                };
                let fetch = manager.retrieve_group_avatar(context);
                let result = tokio::select! {
                    res = fetch => res,
                    _ = wait_for_shutdown(&mut shutdown) => return,
                };
                if let Ok(Some(avatar)) = result {
                    let (avatar_data, checksum) = avatar_cache.prepare_avatar(avatar);
                    let chat_id = group_identifier(&key);
                    sink.emit(Event {
                        kind: EVENT_AVATAR,
                        chat_id: Some(chat_id),
                        title: Some(checksum),
                        data: avatar_data,
                        ..Event::default()
                    });
                }
            }
        }
    }
}

async fn synchronize_groups_task(
    mut manager: Manager<SqliteStore, Registered>,
    sink: EventSink,
    departed_groups: DepartedGroups,
    avatar_cache: AvatarCache,
    metadata_cache: MetadataCache,
    mut shutdown: watch::Receiver<bool>,
    result_tx: tokio_mpsc::Sender<Result<(), String>>,
) {
    let sync = synchronize_and_emit_group_snapshot(
        &mut manager,
        &sink,
        &departed_groups,
        &avatar_cache,
        &metadata_cache,
    );
    tokio::select! {
        result = sync => {
            let _ = result_tx.send(result).await;
        }
        _ = wait_for_shutdown(&mut shutdown) => {}
    }
}

pub(crate) fn spawn_group_sync(
    manager: Manager<SqliteStore, Registered>,
    sink: EventSink,
    departed_groups: DepartedGroups,
    avatar_cache: AvatarCache,
    metadata_cache: MetadataCache,
    shutdown: watch::Receiver<bool>,
    result_tx: tokio_mpsc::Sender<Result<(), String>>,
) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn_local(synchronize_groups_task(
        manager,
        sink,
        departed_groups,
        avatar_cache,
        metadata_cache,
        shutdown,
        result_tx,
    ))
}

pub(crate) async fn handle_attachment_completion(
    manager: &Manager<SqliteStore, Registered>,
    sink: &EventSink,
    attachment_aborts: &mut HashMap<u64, AttachmentTaskControl>,
    completed: Result<AttachmentCompletion, tokio::task::JoinError>,
) {
    if let Some(sent) = finish_attachment_completion(sink, attachment_aborts, completed) {
        let repo = StorageRepository::new(manager.store().clone());
        mark_sent_message_projected_or_report(&repo, &sent, sink).await;
    }
}

pub(crate) fn finish_attachment_completion(
    sink: &EventSink,
    attachment_aborts: &mut HashMap<u64, AttachmentTaskControl>,
    completed: Result<AttachmentCompletion, tokio::task::JoinError>,
) -> Option<SentMessage> {
    let Ok(AttachmentCompletion {
        request_id,
        result,
        permit: _permit,
    }) = completed
    else {
        return None;
    };
    let sent = match result {
        AttachmentTaskResult::Finished(Ok(sent)) => {
            sink.emit(Event {
                kind: EVENT_ATTACHMENT_SENT,
                request_id,
                ..Event::default()
            });
            Some(sent)
        }
        AttachmentTaskResult::Finished(Err(error)) => {
            sink.emit(Event::request_error(request_id, error));
            None
        }
        AttachmentTaskResult::Cancelled => None,
    };
    attachment_aborts.remove(&request_id);
    sent
}

pub(crate) enum AttachmentTaskResult {
    Finished(Result<SentMessage, String>),
    Cancelled,
}

pub(crate) struct AttachmentTaskControl {
    pub(crate) task: tokio::task::AbortHandle,
    pub(crate) control: AttachmentControl,
}

pub(crate) struct AttachmentCompletion {
    pub(crate) request_id: u64,
    pub(crate) result: AttachmentTaskResult,
    pub(crate) permit: AttachmentPermit,
}

pub(crate) struct CommandContext<'a, M, S = StorageRepository> {
    pub(crate) manager: &'a mut M,
    pub(crate) repo: &'a S,
    pub(crate) sink: &'a EventSink,
    pub(crate) departed_groups: &'a DepartedGroups,
    pub(crate) groups_authoritative: bool,
    pub(crate) metadata_cache: &'a MetadataCache,
    pub(crate) timestamps: &'a MessageTimestampAllocator,
}

pub(crate) async fn emit_contact_snapshot(
    manager: &Manager<SqliteStore, Registered>,
    sink: &EventSink,
    avatar_cache: &AvatarCache,
    metadata_cache: &MetadataCache,
) {
    let repo = StorageRepository::new(manager.store().clone());
    match repo.contacts().await {
        Ok(contacts) => {
            sink.emit(Event {
                kind: EVENT_CONTACT_SYNC_BEGIN,
                ..Event::default()
            });
            for (index, contact) in contacts.into_iter().enumerate() {
                if index != 0 && index % SNAPSHOT_YIELD_INTERVAL == 0 {
                    tokio::task::yield_now().await;
                }
                let peer = ServiceId::Aci(contact.uuid.into()).service_id_string();
                let contact_title = if !contact.name.is_empty() {
                    metadata_cache.put_contact_name(peer.clone(), contact.name.clone());
                    Some(contact.name)
                } else {
                    metadata_cache.get_contact_name(&peer)
                };
                sink.emit(Event {
                    kind: EVENT_CONTACT,
                    peer_id: Some(peer.clone()),
                    title: contact_title,
                    text: contact.phone_number.map(|number| number.to_string()),
                    ..Event::default()
                });
                let profile_key = if contact.profile_key.len() == 32 {
                    let mut key_bytes = [0u8; 32];
                    key_bytes.copy_from_slice(&contact.profile_key);
                    Some(ProfileKey::create(key_bytes))
                } else {
                    match repo
                        .contact_profile_key(&ServiceId::Aci(contact.uuid.into()))
                        .await
                    {
                        Ok(key) => key,
                        Err(error) => {
                            if sqlite_store_error_is_transient(&error) {
                                tracing::warn!(
                                    %error,
                                    "Transient store contention reading a Signal contact profile key"
                                );
                            } else {
                                tracing::warn!(%error, "Could not read a Signal contact profile key");
                            }
                            None
                        }
                    }
                };
                let avatar = if let Some(key) = profile_key {
                    match repo.contact_avatar(contact.uuid, key).await {
                        Ok(avatar) => avatar,
                        Err(error) => {
                            if sqlite_store_error_is_transient(&error) {
                                tracing::warn!(
                                    %error,
                                    "Transient store contention reading a cached Signal contact avatar"
                                );
                            } else {
                                tracing::warn!(
                                    %error,
                                    "Could not read a cached Signal contact avatar"
                                );
                            }
                            None
                        }
                    }
                } else {
                    None
                };
                if let Some(avatar) = avatar {
                    let (avatar_data, checksum) = avatar_cache.prepare_avatar(avatar);
                    sink.emit(Event {
                        kind: EVENT_AVATAR,
                        peer_id: Some(peer),
                        title: Some(checksum),
                        data: avatar_data,
                        ..Event::default()
                    });
                }
            }
            sink.emit(Event {
                kind: EVENT_CONTACT_SYNC_END,
                ..Event::default()
            });
        }
        Err(error) if error.is_transient() => {
            sink.emit(Event::transient_error(format!(
                "Could not read synchronized Signal contacts: {error}"
            )));
        }
        Err(error) => {
            sink.emit(Event::error(
                format!("Could not read synchronized Signal contacts: {error}"),
                false,
            ));
        }
    }
}

fn account_identity_event(aci: Aci, profile_name: Option<String>) -> Event {
    Event {
        kind: EVENT_ACCOUNT,
        peer_id: Some(ServiceId::Aci(aci).service_id_string()),
        title: profile_name.filter(|name| !name.is_empty()),
        ..Event::default()
    }
}

pub(crate) async fn emit_account_identity(
    manager: &mut Manager<SqliteStore, Registered>,
    sink: &EventSink,
) {
    let local_aci = manager.registration_data().service_ids.aci();
    let profile_name = manager
        .retrieve_profile()
        .await
        .ok()
        .and_then(|profile| profile.name)
        .map(|name| name.to_string());

    sink.emit(account_identity_event(local_aci, profile_name));
}

pub(crate) async fn emit_group_snapshot(
    manager: &Manager<SqliteStore, Registered>,
    sink: &EventSink,
    departed_groups: &DepartedGroups,
    avatar_cache: &AvatarCache,
    metadata_cache: &MetadataCache,
    authoritative: bool,
) -> Result<(), String> {
    let repo = StorageRepository::new(manager.store().clone());
    let groups = repo.groups().await.map_err(|error| error.to_string())?;

    sink.emit(Event {
        kind: EVENT_GROUP_SYNC_BEGIN,
        ..Event::default()
    });
    let local_aci = manager.registration_data().service_ids.aci();
    let mut emitted_records = 0;
    for (key, group) in groups {
        if emitted_records != 0 && emitted_records % SNAPSHOT_YIELD_INTERVAL == 0 {
            tokio::task::yield_now().await;
        }
        let chat_id = group_identifier(&key);
        if departed_groups.contains(&chat_id) || !group_contains_local_aci(&group, &local_aci) {
            continue;
        }
        metadata_cache.index_group(chat_id.clone(), key);
        sink.emit(Event {
            kind: EVENT_GROUP,
            chat_id: Some(chat_id.clone()),
            title: Some(group.title),
            ..Event::default()
        });
        emitted_records += 1;
        let avatar = match repo.group_avatar(key).await {
            Ok(avatar) => avatar,
            Err(error) => {
                if sqlite_store_error_is_transient(&error) {
                    tracing::warn!(%error, "Transient store contention reading a Signal group avatar");
                } else {
                    tracing::warn!(%error, "Could not read a Signal group avatar");
                }
                None
            }
        };
        if let Some(avatar) = avatar {
            let (avatar_data, checksum) = avatar_cache.prepare_avatar(avatar);
            sink.emit(Event {
                kind: EVENT_AVATAR,
                chat_id: Some(chat_id.clone()),
                title: Some(checksum),
                data: avatar_data,
                ..Event::default()
            });
            emitted_records += 1;
        }
        for member in group.members {
            if emitted_records % SNAPSHOT_YIELD_INTERVAL == 0 {
                tokio::task::yield_now().await;
            }
            sink.emit(Event {
                kind: EVENT_GROUP_MEMBER,
                chat_id: Some(chat_id.clone()),
                peer_id: Some(ServiceId::Aci(member.aci).service_id_string()),
                value: i32::from(member.role == Role::Administrator),
                ..Event::default()
            });
            emitted_records += 1;
        }
    }
    sink.emit(Event {
        kind: EVENT_GROUP_SYNC_END,
        value: i32::from(authoritative),
        ..Event::default()
    });
    Ok(())
}

async fn synchronize_and_emit_group_snapshot(
    manager: &mut Manager<SqliteStore, Registered>,
    sink: &EventSink,
    departed_groups: &DepartedGroups,
    avatar_cache: &AvatarCache,
    metadata_cache: &MetadataCache,
) -> Result<(), String> {
    manager
        .synchronize_storage_groups()
        .await
        .map_err(|error| format!("Could not synchronize Signal groups: {error}"))?;
    emit_group_snapshot(
        manager,
        sink,
        departed_groups,
        avatar_cache,
        metadata_cache,
        true,
    )
    .await
}

pub(crate) async fn emit_identity_changes(
    manager: &Manager<SqliteStore, Registered>,
    sink: &EventSink,
) {
    let repo = StorageRepository::new(manager.store().clone());
    match repo.identity_change_notices().await {
        Ok(changes) => {
            for (index, change) in changes.into_iter().enumerate() {
                if index != 0 && index % SNAPSHOT_YIELD_INTERVAL == 0 {
                    tokio::task::yield_now().await;
                }
                sink.emit(Event {
                    kind: EVENT_IDENTITY_CHANGE,
                    peer_id: Some(change.address),
                    value: i32::from(change.verified),
                    ..Event::default()
                });
            }
        }
        Err(error) => {
            if sqlite_store_error_is_transient(&error) {
                tracing::warn!(%error, "Transient store contention reading Signal identity changes; deferring");
            } else {
                sink.emit(Event::error(
                    format!("Could not read Signal identity changes: {error}"),
                    false,
                ));
            }
        }
    }
}

/// Short bounded backoff for a transient failure or a not-yet-visible row when marking a
/// just-sent message as already projected. Without this, a passing SQLite BUSY/lock error
/// or a read racing the write that just committed permanently loses the projection marker:
/// the message would then replay to the UI as newly received on the next start.
const MARK_SENT_PROJECTED_RETRY_DELAYS_MS: [u64; 3] = [50, 200, 500];

async fn mark_sent_message_projected(
    repo: &impl StorageOps,
    sent: &SentMessage,
) -> Result<(), StorageError> {
    let mut attempt = 0;
    loop {
        match repo
            .mark_sent_message_projected(&sent.thread, sent.timestamp)
            .await
        {
            Ok(()) => return Ok(()),
            Err(error) if error.is_transient() => {
                let Some(delay) = MARK_SENT_PROJECTED_RETRY_DELAYS_MS.get(attempt) else {
                    return Err(error);
                };
                tokio::time::sleep(Duration::from_millis(*delay)).await;
                attempt += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

pub(crate) async fn mark_sent_message_projected_or_report(
    repo: &impl StorageOps,
    sent: &SentMessage,
    sink: &EventSink,
) {
    if let Err(error) = mark_sent_message_projected(repo, sent).await {
        sink.emit(Event::error(error.to_string(), false));
    }
}

pub(crate) struct OutgoingAttachment {
    pub(crate) recipient: String,
    pub(crate) filename: String,
    pub(crate) content_type: String,
    pub(crate) data: AttachmentPayload,
    pub(crate) group: bool,
}

pub(crate) async fn upload_and_send_attachment<M: SignalProtocol>(
    manager: &mut M,
    repo: &impl StorageOps,
    attachment: OutgoingAttachment,
    departed_groups: &DepartedGroups,
    metadata_cache: &MetadataCache,
    timestamps: &MessageTimestampAllocator,
) -> Result<SentMessage, String> {
    let OutgoingAttachment {
        recipient,
        filename,
        content_type,
        data,
        group,
    } = attachment;
    let data = match data {
        AttachmentPayload::Data(bytes) => bytes,
        AttachmentPayload::Path(path) => tokio::fs::read(&path)
            .await
            .map_err(|error| format!("Could not read attachment file: {error}"))?,
    };
    if data.is_empty() || data.len() > MAX_ATTACHMENT_BYTES {
        return Err("Attachment size is outside the supported range".into());
    }
    let group_target = if group {
        Some(
            resolve_active_group(manager, repo, &recipient, departed_groups, metadata_cache)
                .await?
                .ok_or_else(|| {
                    "Signal group is unavailable or this account is no longer a member".to_owned()
                })?,
        )
    } else {
        None
    };
    let pointer = manager
        .upload_attachment(
            AttachmentSpec {
                content_type,
                length: data.len(),
                file_name: Some(filename),
                preview: None,
                voice_note: None,
                borderless: None,
                width: None,
                height: None,
                caption: None,
                blur_hash: None,
            },
            data,
        )
        .await?;
    let timestamp = timestamps.next();
    match group_target {
        Some((key, _)) => {
            let _operation = departed_groups.lock_operation().await;
            let group = active_group_by_key(manager, repo, key, departed_groups)
                .await?
                .ok_or_else(|| {
                    "Signal group became unavailable before the attachment could be sent".to_owned()
                })?;
            manager
                .send_message_to_group(
                    &key,
                    DataMessage {
                        attachments: vec![pointer],
                        timestamp: Some(timestamp),
                        group_v2: Some(GroupContextV2 {
                            master_key: Some(key.to_vec()),
                            revision: Some(group.revision),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }
                    .into(),
                    timestamp,
                )
                .await?;
            Ok(SentMessage {
                thread: Thread::Group(key),
                timestamp,
            })
        }
        None => {
            let recipient = parse_recipient(&recipient).ok_or_else(|| {
                "Recipient is not a canonical Signal service identifier".to_owned()
            })?;
            manager
                .send_message(
                    recipient,
                    DataMessage {
                        attachments: vec![pointer],
                        timestamp: Some(timestamp),
                        ..Default::default()
                    }
                    .into(),
                    timestamp,
                )
                .await?;
            Ok(SentMessage {
                thread: Thread::Contact(recipient),
                timestamp,
            })
        }
    }
}

pub(crate) async fn load_unprojected_messages(
    manager: &Manager<SqliteStore, Registered>,
    sink: &EventSink,
    replay: &mut MessageReplayQueue,
    groups_authoritative: bool,
) {
    let repo = StorageRepository::new(manager.store().clone());
    let messages = match repo.unprojected_messages().await {
        Ok(messages) => messages,
        Err(error) => {
            if sqlite_store_error_is_transient(&error) {
                tracing::warn!(%error, "Transient store contention reading pending Signal messages; deferring");
            } else {
                sink.emit(Event::error(
                    format!("Could not read pending Signal messages: {error}"),
                    false,
                ));
            }
            return;
        }
    };

    replay.replace(messages, groups_authoritative);
}

pub(crate) async fn handle_command<M: SignalProtocol, S: StorageOps>(
    ctx: CommandContext<'_, M, S>,
    command: Command,
) {
    let CommandContext {
        manager,
        repo,
        sink,
        departed_groups,
        groups_authoritative,
        metadata_cache,
        timestamps,
    } = ctx;
    match command {
        Command::AcceptIdentity {
            request_id,
            recipient,
        } => match repo.accept_identity_change(&recipient).await {
            Ok(true) => {
                if let Err(error) = repo.expedite_outbox_messages(&recipient).await {
                    sink.emit(Event::error(
                        format!("Could not expedite queued Signal messages: {error}"),
                        false,
                    ));
                }
                sink.emit(Event {
                    kind: EVENT_IDENTITY_ACCEPTED,
                    request_id,
                    peer_id: Some(recipient),
                    ..Event::default()
                });
                retry_outbox(
                    manager,
                    repo,
                    sink,
                    departed_groups,
                    metadata_cache,
                    groups_authoritative,
                )
                .await;
            }
            Ok(false) => sink.emit(Event::request_error(
                request_id,
                "No verified identity change is pending for this contact",
            )),
            Err(error) => sink.emit(Event::request_error(
                request_id,
                format!("Could not accept the Signal identity change: {error}"),
            )),
        },
        Command::DismissIdentity {
            request_id,
            recipient,
        } => match repo.dismiss_identity_change(&recipient).await {
            Ok(()) => {
                sink.emit(Event {
                    kind: EVENT_IDENTITY_CHANGE,
                    request_id,
                    peer_id: Some(recipient),
                    ..Event::default()
                });
            }
            Err(error) => sink.emit(Event::request_error(
                request_id,
                format!("Could not dismiss the Signal identity notice: {error}"),
            )),
        },
        Command::ResetSession {
            request_id,
            recipient,
        } => {
            let result = match parse_recipient(&recipient) {
                Some(recipient) => manager.clear_sessions(&recipient).await,
                None => Err("Recipient is not a canonical Signal service identifier".into()),
            };
            match result {
                Ok(()) => {
                    sink.emit(Event {
                        kind: EVENT_SESSION_RESET,
                        request_id,
                        peer_id: Some(recipient),
                        ..Event::default()
                    });
                }
                Err(error) => {
                    sink.emit(Event::request_error(
                        request_id,
                        format!("Could not reset the Signal session: {error}"),
                    ));
                }
            }
        }
        Command::MarkRead {
            request_id,
            recipient,
            timestamp,
        } => {
            let result = match parse_recipient(&recipient) {
                Some(recipient) => {
                    let send_timestamp = timestamps.next();
                    manager
                        .send_message(
                            recipient,
                            ReceiptMessage {
                                r#type: Some(receipt_message::Type::Read.into()),
                                timestamp: vec![timestamp],
                            }
                            .into(),
                            send_timestamp,
                        )
                        .await
                }
                None => Err("Recipient is not a canonical Signal service identifier".into()),
            };
            if let Err(error) = result {
                sink.emit(Event::transient_request_error(request_id, error));
            }
        }
        Command::LeaveGroup {
            request_id,
            group_key,
        } => {
            if !groups_authoritative {
                departed_groups.cancel_leave(&group_key);
                sink.emit(Event::group_request_error(
                    request_id,
                    group_key,
                    "Signal groups are temporarily unavailable until authoritative synchronization succeeds",
                ));
                return;
            }
            let group_operation = departed_groups.lock_operation().await;
            let resolved = resolve_active_group_for_leave(
                manager,
                repo,
                &group_key,
                departed_groups,
                metadata_cache,
            )
            .await;
            let Some((key, _)) = (match resolved {
                Ok(group) => group,
                Err(error) => {
                    departed_groups.cancel_leave(&group_key);
                    sink.emit(Event::group_request_error(request_id, group_key, error));
                    return;
                }
            }) else {
                departed_groups.cancel_leave(&group_key);
                sink.emit(Event::group_request_error(
                    request_id,
                    group_key,
                    "Signal group is unavailable or this account is no longer a member",
                ));
                return;
            };

            match manager.leave_group(&key).await {
                Ok(outcome) => {
                    for event in group_leave_completion_events(
                        departed_groups,
                        request_id,
                        &group_key,
                        GroupLeaveCompletion::Accepted {
                            peer_notification_sent: outcome.peer_notification_sent,
                            local_group_removed: outcome.local_group_removed,
                        },
                    ) {
                        sink.emit(event);
                    }
                    drop(group_operation);
                    metadata_cache.remove_group_index(&group_key);
                    if let Err(error) = repo.expedite_outbox_messages(&group_key).await {
                        sink.emit(Event::error(
                            format!("Could not schedule stale group messages for cleanup: {error}"),
                            false,
                        ));
                    }
                    retry_outbox(
                        manager,
                        repo,
                        sink,
                        departed_groups,
                        metadata_cache,
                        groups_authoritative,
                    )
                    .await;
                }
                Err(error) => {
                    for event in group_leave_completion_events(
                        departed_groups,
                        request_id,
                        &group_key,
                        GroupLeaveCompletion::Failed(format!(
                            "Could not leave the Signal group: {error}"
                        )),
                    ) {
                        sink.emit(event);
                    }
                }
            }
        }
        Command::SendMessage {
            request_id,
            recipient,
            message,
        } => {
            let result = if parse_recipient(&recipient).is_some() {
                enqueue_and_send(
                    manager,
                    repo,
                    NewOutboxMessage {
                        kind: ClientOutboxKind::Direct,
                        recipient,
                        body: message,
                    },
                    departed_groups,
                    metadata_cache,
                    sink,
                    timestamps,
                )
                .await
            } else {
                Err("Recipient is not a canonical Signal service identifier".into())
            };
            if let Err(error) = result {
                sink.emit(Event::transient_request_error(request_id, error));
            }
        }
        Command::SendGroupMessage {
            request_id,
            group_key,
            message,
        } => {
            let result = if !groups_authoritative {
                Err(
                    "Signal groups are temporarily unavailable until authoritative synchronization succeeds"
                        .into(),
                )
            } else {
                match resolve_active_group(
                    manager,
                    repo,
                    &group_key,
                    departed_groups,
                    metadata_cache,
                )
                .await
                {
                    Ok(Some(_)) => {
                        enqueue_and_send(
                            manager,
                            repo,
                            NewOutboxMessage {
                                kind: ClientOutboxKind::Group,
                                recipient: group_key,
                                body: message,
                            },
                            departed_groups,
                            metadata_cache,
                            sink,
                            timestamps,
                        )
                        .await
                    }
                    Ok(None) => Err(
                        "Signal group is unavailable or this account is no longer a member".into(),
                    ),
                    Err(error) => Err(error),
                }
            };
            if let Err(error) = result {
                sink.emit(Event::transient_request_error(request_id, error));
            }
        }
        Command::SetTyping {
            request_id,
            recipient,
            typing,
        } => {
            let result = match parse_recipient(&recipient) {
                Some(recipient) => {
                    let timestamp = timestamps.next();
                    manager
                        .send_message(
                            recipient,
                            TypingMessage {
                                timestamp: Some(timestamp),
                                action: Some(if typing {
                                    typing_message::Action::Started.into()
                                } else {
                                    typing_message::Action::Stopped.into()
                                }),
                                group_id: None,
                            }
                            .into(),
                            timestamp,
                        )
                        .await
                }
                None => Err("Recipient is not a canonical Signal service identifier".into()),
            };
            if let Err(error) = result {
                sink.emit(Event::transient_request_error(request_id, error));
            }
        }
        Command::SendAttachment { .. } => {
            unreachable!(
                "SendAttachment is routed directly by the worker loop and never reaches handle_command"
            )
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_content<M: SignalProtocol, S: StorageOps>(
    manager: &mut M,
    repo: &S,
    content: Content,
    delivery_id: u64,
    sink: &EventSink,
    departed_groups: &DepartedGroups,
    delivery_receipts: &mut DeliveryReceiptQueue,
    timestamps: &MessageTimestampAllocator,
) -> ProjectionDisposition {
    let timestamp = content_timestamp(&content);
    let sender = content.metadata.sender.service_id_string();
    let local_aci = manager.local_aci();

    match &content.body {
        ContentBody::DataMessage(message) => {
            let route = bare_data_message_route(
                content.metadata.sender,
                content.metadata.destination,
                local_aci,
            );
            let projection = if route.outgoing {
                DataMessageProjection::outgoing(message, &route.peer, timestamp, delivery_id)
            } else {
                DataMessageProjection::incoming(message, &route.peer, timestamp, delivery_id)
            };
            let disposition =
                emit_data_message(manager, repo, projection, sink, departed_groups).await;
            if content.metadata.needs_receipt
                && delivery_receipts.enqueue(content.metadata.sender, timestamp, timestamps.next())
            {
                sink.emit(Event::transient_error(
                    "Signal delivery-receipt backlog limit reached; excess receipts were discarded",
                ));
            }
            return disposition;
        }
        ContentBody::EditMessage(EditMessage {
            data_message: Some(message),
            ..
        }) => {
            let route = bare_data_message_route(
                content.metadata.sender,
                content.metadata.destination,
                local_aci,
            );
            let projection = if route.outgoing {
                DataMessageProjection::outgoing(message, &route.peer, timestamp, delivery_id)
            } else {
                DataMessageProjection::incoming(message, &route.peer, timestamp, delivery_id)
            };
            return emit_data_message(manager, repo, projection, sink, departed_groups).await;
        }
        ContentBody::SynchronizeMessage(SyncMessage {
            sent: Some(sent), ..
        }) => {
            if let Some(message) = sent.message.as_ref() {
                let peer = sent
                    .parse_destination_service_id()
                    .map_or_else(|| sender.clone(), |id| id.service_id_string());
                return emit_data_message(
                    manager,
                    repo,
                    DataMessageProjection::outgoing(message, &peer, timestamp, delivery_id),
                    sink,
                    departed_groups,
                )
                .await;
            } else if let Some(EditMessage {
                data_message: Some(message),
                ..
            }) = sent.edit_message.as_ref()
            {
                let peer = sent
                    .parse_destination_service_id()
                    .map_or_else(|| sender.clone(), |id| id.service_id_string());
                return emit_data_message(
                    manager,
                    repo,
                    DataMessageProjection::outgoing(message, &peer, timestamp, delivery_id),
                    sink,
                    departed_groups,
                )
                .await;
            }
        }
        ContentBody::TypingMessage(message) if message.group_id.is_none() => {
            let started = message.action == Some(typing_message::Action::Started.into());
            sink.emit(Event {
                kind: EVENT_TYPING,
                peer_id: Some(sender),
                timestamp_ms: message.timestamp.unwrap_or(timestamp),
                value: i32::from(started),
                ..Event::default()
            });
        }
        ContentBody::ReceiptMessage(message) => {
            sink.emit(Event {
                kind: EVENT_RECEIPT,
                peer_id: Some(sender),
                timestamp_ms: message.timestamp.first().copied().unwrap_or(timestamp),
                value: message.r#type.unwrap_or_default(),
                ..Event::default()
            });
        }
        ContentBody::DecryptionErrorMessage(_) => sink.emit(Event::error(
            format!("A message from {sender} could not be decrypted"),
            false,
        )),
        _ => {}
    }
    ProjectionDisposition::Complete
}

struct DataMessageProjection<'a> {
    message: &'a DataMessage,
    peer: &'a str,
    outgoing: bool,
    timestamp: u64,
    delivery_id: u64,
}

impl<'a> DataMessageProjection<'a> {
    fn incoming(message: &'a DataMessage, peer: &'a str, timestamp: u64, delivery_id: u64) -> Self {
        Self {
            message,
            peer,
            outgoing: false,
            timestamp,
            delivery_id,
        }
    }

    fn outgoing(message: &'a DataMessage, peer: &'a str, timestamp: u64, delivery_id: u64) -> Self {
        Self {
            message,
            peer,
            outgoing: true,
            timestamp,
            delivery_id,
        }
    }
}

fn data_message_text(message: &DataMessage) -> String {
    if let Some(reaction) = &message.reaction
        && let Some(emoji) = &reaction.emoji
    {
        return format!("Reacted with {emoji}");
    }
    if let Some(body) = message.body.as_deref().filter(|body| !body.is_empty()) {
        return body.to_owned();
    }
    message
        .preview
        .iter()
        .find_map(|preview| {
            preview
                .url
                .as_deref()
                .filter(|text| !text.is_empty())
                .or_else(|| preview.title.as_deref().filter(|text| !text.is_empty()))
                .or_else(|| {
                    preview
                        .description
                        .as_deref()
                        .filter(|text| !text.is_empty())
                })
        })
        .unwrap_or_default()
        .to_owned()
}

fn regular_message_attachments(message: &DataMessage) -> &[AttachmentPointer] {
    &message.attachments
}

fn attachment_pointer_without_sender_size_hint(
    attachment: &AttachmentPointer,
) -> AttachmentPointer {
    // The sender controls this field, and Presage otherwise uses it as the
    // initial allocation before reading the authenticated network body.
    let mut download_pointer = attachment.clone();
    download_pointer.size = None;
    download_pointer
}

fn truncate_attachment_to_sender_size(attachment: &AttachmentPointer, data: &mut Vec<u8>) {
    if let Some(size) = attachment.size.and_then(|size| size.try_into().ok()) {
        // Preserve Presage's post-decryption privacy-padding behavior without
        // trusting the sender-provided size for allocation.
        data.truncate(size);
    }
}

fn projected_data_message_text<'a>(
    mut text: String,
    attachments: impl IntoIterator<Item = (Option<&'a str>, bool)>,
) -> Option<String> {
    for (name, suppress_placeholder) in attachments {
        if suppress_placeholder {
            continue;
        }
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&format!("[Attachment: {}]", name.unwrap_or("attachment")));
    }
    (!text.is_empty()).then_some(text)
}

async fn emit_data_message<M: SignalProtocol, S: StorageOps>(
    manager: &M,
    repo: &S,
    projection: DataMessageProjection<'_>,
    sink: &EventSink,
    departed_groups: &DepartedGroups,
) -> ProjectionDisposition {
    let DataMessageProjection {
        message,
        peer,
        outgoing,
        timestamp,
        delivery_id,
    } = projection;
    let target = group_message_target(message);
    if target == GroupMessageTarget::Malformed {
        sink.emit(Event::error(
            "Ignored a Signal group message with a missing or malformed group master key",
            false,
        ));
        return ProjectionDisposition::Complete;
    }

    let text = data_message_text(message);
    let attachments = regular_message_attachments(message);
    if text.is_empty() && attachments.is_empty() {
        return ProjectionDisposition::Complete;
    }

    let group_key = match target {
        GroupMessageTarget::Direct => None,
        GroupMessageTarget::Group(key) => Some(key),
        GroupMessageTarget::Malformed => unreachable!(),
    };
    let group_title = if let Some(group_key) = group_key {
        match group_for_projection(manager, repo, group_key, departed_groups).await {
            Ok(ProjectionGroup::Active(group)) => Some(group.title),
            Ok(ProjectionGroup::Complete) => return ProjectionDisposition::Complete,
            Ok(ProjectionGroup::Retry) => return ProjectionDisposition::Retry,
            Err(error) => {
                sink.emit(Event::error(error, false));
                return ProjectionDisposition::Retry;
            }
        }
    } else {
        None
    };
    let flags = if outgoing { FLAG_OUTGOING } else { 0 };
    let mut downloaded = Vec::new();
    if !outgoing {
        if attachments.len() > MAX_ATTACHMENT_DOWNLOADS_PER_MESSAGE {
            sink.emit(Event::error(
                format!(
                    "Ignored {} Signal attachments beyond the per-message limit of {MAX_ATTACHMENT_DOWNLOADS_PER_MESSAGE}",
                    attachments.len() - MAX_ATTACHMENT_DOWNLOADS_PER_MESSAGE
                ),
                false,
            ));
        }
        for (attachment_index, attachment) in attachments
            .iter()
            .enumerate()
            .take(MAX_ATTACHMENT_DOWNLOADS_PER_MESSAGE)
        {
            let download_pointer = attachment_pointer_without_sender_size_hint(attachment);
            match manager.get_attachment(&download_pointer).await {
                Ok(mut data) => {
                    truncate_attachment_to_sender_size(attachment, &mut data);
                    if data.is_empty() {
                        sink.emit(Event::error(
                            "Could not download a Signal attachment: decrypted attachment was empty",
                            false,
                        ));
                    } else {
                        downloaded.push(DownloadedAttachment::new(
                            attachment_index,
                            attachment,
                            data,
                        ));
                    }
                }
                Err(error) => sink.emit(Event::error(
                    format!("Could not download a Signal attachment: {error}"),
                    false,
                )),
            }
        }
    }

    let mut signal_gif_transcodes = 0usize;
    for attachment in &mut downloaded {
        if signal_gif_transcodes >= MAX_SIGNAL_GIF_TRANSCODES_PER_MESSAGE {
            break;
        }
        if attachment.signal_gif_filename.is_none() {
            continue;
        }
        signal_gif_transcodes += 1;
        if let Some(gif) = transcode_signal_gif_video(&attachment.data).await {
            attachment.apply_signal_gif(gif);
        }
    }

    let inline_attachment_indexes: HashSet<usize> = downloaded
        .iter()
        .filter_map(|attachment| {
            should_inline_image(
                outgoing,
                attachment.content_type.as_deref(),
                Some(&attachment.data),
            )
            .then_some(attachment.attachment_index)
        })
        .collect();
    let text = projected_data_message_text(
        text,
        attachments
            .iter()
            .enumerate()
            .map(|(attachment_index, attachment)| {
                (
                    Some(attachment_display_name(attachment)),
                    inline_attachment_indexes.contains(&attachment_index),
                )
            }),
    );

    let message_delivery_id = if downloaded.is_empty() {
        delivery_id
    } else {
        0
    };

    if let (Some(group_key), Some(text)) = (group_key, text.as_ref()) {
        let group_peer = group_message_peer(outgoing, peer, manager.local_aci());
        sink.emit(Event {
            kind: EVENT_GROUP_MESSAGE,
            request_id: message_delivery_id,
            flags,
            peer_id: Some(group_peer),
            chat_id: Some(group_identifier(&group_key)),
            title: group_title,
            text: Some(text.clone()),
            timestamp_ms: timestamp,
            ..Event::default()
        });
    } else if let Some(text) = text {
        sink.emit(Event {
            kind: EVENT_MESSAGE,
            request_id: message_delivery_id,
            flags,
            peer_id: Some(peer.to_owned()),
            text: Some(text),
            timestamp_ms: timestamp,
            ..Event::default()
        });
    }

    let attachment_count = downloaded.len();
    for (index, attachment) in downloaded.into_iter().enumerate() {
        sink.emit(Event {
            kind: EVENT_ATTACHMENT,
            request_id: if index + 1 == attachment_count {
                delivery_id
            } else {
                0
            },
            peer_id: Some(peer.to_owned()),
            chat_id: group_key.map(|key| group_identifier(&key)),
            title: Some(attachment.filename),
            text: attachment.content_type,
            data: attachment.data,
            timestamp_ms: timestamp,
            ..Event::default()
        });
    }
    ProjectionDisposition::AwaitingAck
}

pub(crate) fn parse_recipient(value: &str) -> Option<ServiceId> {
    ServiceId::parse_from_service_id_string(value).or_else(|| {
        value
            .parse::<presage::libsignal_service::prelude::Uuid>()
            .ok()
            .map(|uuid| ServiceId::Aci(uuid.into()))
    })
}

fn group_identifier(group_key: &[u8; 32]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"signal-purple group identifier\0");
    digest.update(group_key);
    hex::encode(digest.finalize())
}

fn group_leave_warning_messages(
    peer_notification_sent: bool,
    local_group_removed: bool,
) -> Vec<&'static str> {
    let mut warnings = Vec::new();
    if !peer_notification_sent {
        warnings.push(
            "Signal accepted the group leave, but some remaining members could not be notified",
        );
    }
    if !local_group_removed {
        warnings.push(
            "Signal accepted the group leave, but the encrypted local group cache could not be removed; reconnect to retry cleanup",
        );
    }
    warnings
}

fn contains_local_aci<'a>(mut members: impl Iterator<Item = &'a Aci>, local_aci: &Aci) -> bool {
    members.any(|member| member == local_aci)
}

fn group_contains_local_aci(group: &Group, local_aci: &Aci) -> bool {
    contains_local_aci(group.members.iter().map(|member| &member.aci), local_aci)
}

async fn group_for_projection<M: SignalProtocol, S: StorageOps>(
    manager: &M,
    repo: &S,
    key: [u8; 32],
    departed_groups: &DepartedGroups,
) -> Result<ProjectionGroup, String> {
    let identifier = group_identifier(&key);
    if let Some(disposition) =
        departure_projection_disposition(departed_groups.departure_state(&identifier))
    {
        return Ok(match disposition {
            ProjectionDisposition::Retry => ProjectionGroup::Retry,
            ProjectionDisposition::Complete => ProjectionGroup::Complete,
            ProjectionDisposition::AwaitingAck => unreachable!(),
        });
    }

    let group = repo
        .group(key)
        .await
        .map_err(|error| format!("Could not read Signal group membership: {error}"))?;

    if let Some(disposition) =
        departure_projection_disposition(departed_groups.departure_state(&identifier))
    {
        return Ok(match disposition {
            ProjectionDisposition::Retry => ProjectionGroup::Retry,
            ProjectionDisposition::Complete => ProjectionGroup::Complete,
            ProjectionDisposition::AwaitingAck => unreachable!(),
        });
    }

    let local_aci = manager.local_aci();
    Ok(
        match group.filter(|group| group_contains_local_aci(group, &local_aci)) {
            Some(group) => ProjectionGroup::Active(group),
            None => ProjectionGroup::Complete,
        },
    )
}

async fn active_group_by_key<M: SignalProtocol, S: StorageOps>(
    manager: &M,
    repo: &S,
    key: [u8; 32],
    departed_groups: &DepartedGroups,
) -> Result<Option<Group>, String> {
    if departed_groups.contains(&group_identifier(&key)) {
        return Ok(None);
    }
    let local_aci = manager.local_aci();
    repo.active_group(key, &local_aci)
        .await
        .map_err(|error| error.to_string())
}

pub(crate) async fn resolve_active_group<M: SignalProtocol, S: StorageOps>(
    manager: &M,
    repo: &S,
    identifier: &str,
    departed_groups: &DepartedGroups,
    metadata_cache: &MetadataCache,
) -> Result<Option<([u8; 32], Group)>, String> {
    if departed_groups.contains(identifier) {
        return Ok(None);
    }
    resolve_active_group_in_store(manager, repo, identifier, metadata_cache).await
}

async fn resolve_active_group_for_leave<M: SignalProtocol, S: StorageOps>(
    manager: &M,
    repo: &S,
    identifier: &str,
    departed_groups: &DepartedGroups,
    metadata_cache: &MetadataCache,
) -> Result<Option<([u8; 32], Group)>, String> {
    if departed_groups.is_departed(identifier) {
        return Ok(None);
    }
    resolve_active_group_in_store(manager, repo, identifier, metadata_cache).await
}

/// Resolves a group by its hashed identifier. Checks `metadata_cache`'s
/// identifier index first (an O(1) lookup plus one single-group store read)
/// before falling back to a full scan of every group, which would otherwise
/// run on every group send, leave, and outbox retry attempt.
async fn resolve_active_group_in_store<M: SignalProtocol, S: StorageOps>(
    manager: &M,
    repo: &S,
    identifier: &str,
    metadata_cache: &MetadataCache,
) -> Result<Option<([u8; 32], Group)>, String> {
    let local_aci = manager.local_aci();
    if let Some(key) = metadata_cache.group_key_for_identifier(identifier) {
        let indexed = repo
            .active_group(key, &local_aci)
            .await
            .map_err(|error| error.to_string())?;
        if let Some(group) = indexed {
            return Ok(Some((key, group)));
        }
    }
    let groups = repo.groups().await.map_err(|error| error.to_string())?;
    let found = groups.into_iter().find(|(key, group)| {
        group_identifier(key) == identifier && group_contains_local_aci(group, &local_aci)
    });
    if let Some((key, _)) = &found {
        metadata_cache.index_group(identifier.to_owned(), *key);
    }
    Ok(found)
}

fn content_timestamp(content: &Content) -> u64 {
    match &content.body {
        ContentBody::DataMessage(DataMessage {
            timestamp: Some(timestamp),
            ..
        }) => *timestamp,
        ContentBody::EditMessage(EditMessage {
            target_sent_timestamp: Some(timestamp),
            ..
        }) => *timestamp,
        ContentBody::SynchronizeMessage(SyncMessage {
            sent: Some(sent), ..
        }) => sent
            .timestamp
            .unwrap_or_else(|| content.metadata.timestamp.timestamp_millis() as u64),
        _ => content.metadata.timestamp.timestamp_millis() as u64,
    }
}

pub(crate) fn wall_clock_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub(crate) fn qr_png(value: &[u8]) -> Result<Vec<u8>, String> {
    const BORDER_MODULES: usize = 4;
    const SCALE: usize = 6;

    let code = QrCode::new(value).map_err(|error| error.to_string())?;
    let modules = code.width();
    let pixels_wide = (modules + BORDER_MODULES * 2) * SCALE;
    let mut pixels = vec![255u8; pixels_wide * pixels_wide];

    for y in 0..modules {
        for x in 0..modules {
            if code[(x, y)] != Color::Dark {
                continue;
            }
            let start_x = (x + BORDER_MODULES) * SCALE;
            let start_y = (y + BORDER_MODULES) * SCALE;
            for pixel_y in start_y..start_y + SCALE {
                for pixel_x in start_x..start_x + SCALE {
                    pixels[pixel_y * pixels_wide + pixel_x] = 0;
                }
            }
        }
    }

    let mut output = Vec::new();
    let mut encoder = png::Encoder::new(&mut output, pixels_wide as u32, pixels_wide as u32);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer
        .write_image_data(&pixels)
        .map_err(|error| error.to_string())?;
    drop(writer);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_timestamps_advance_when_wall_clock_stalls() {
        let timestamps = MessageTimestampAllocator::default();

        assert_eq!(timestamps.next_at(1_000), 1_000);
        assert_eq!(timestamps.next_at(1_000), 1_001);
        assert_eq!(timestamps.next_at(1_000), 1_002);
    }

    #[test]
    fn message_timestamps_advance_when_wall_clock_moves_backwards() {
        let timestamps = MessageTimestampAllocator::default();

        assert_eq!(timestamps.next_at(2_000), 2_000);
        assert_eq!(timestamps.next_at(1_000), 2_001);
    }

    #[test]
    fn message_timestamp_clones_share_one_concurrent_sequence() {
        const WORKERS: u64 = 8;
        const ALLOCATIONS_PER_WORKER: u64 = 128;

        let timestamps = MessageTimestampAllocator::default();
        let handles = (0..WORKERS)
            .map(|_| {
                let timestamps = timestamps.clone();
                std::thread::spawn(move || {
                    (0..ALLOCATIONS_PER_WORKER)
                        .map(|_| timestamps.next_at(10_000))
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        let mut allocated = handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("timestamp worker panicked"))
            .collect::<Vec<_>>();

        allocated.sort_unstable();
        assert_eq!(
            allocated,
            (10_000..10_000 + WORKERS * ALLOCATIONS_PER_WORKER).collect::<Vec<_>>()
        );
    }

    #[test]
    fn derives_stable_non_secret_group_identifiers() {
        let first = group_identifier(&[0; 32]);
        let second = group_identifier(&[1; 32]);

        assert_eq!(first.len(), 64);
        assert_eq!(
            first,
            "3560c18a595af2d16e2297a210a7b429779e0da6d83411193cf692b4a1e137d7"
        );
        assert_ne!(first, hex::encode([0; 32]));
        assert_ne!(first, second);
    }

    #[test]
    fn projects_the_local_account_identity_and_optional_profile_name() {
        let Some(ServiceId::Aci(local)) =
            ServiceId::parse_from_service_id_string("11111111-1111-4111-8111-111111111111")
        else {
            panic!("test ACI must parse");
        };

        let named = account_identity_event(local, Some("Signal Profile".into()));
        assert_eq!(named.kind, EVENT_ACCOUNT);
        assert_eq!(
            named.peer_id.as_deref(),
            Some("11111111-1111-4111-8111-111111111111")
        );
        assert_eq!(named.title.as_deref(), Some("Signal Profile"));

        let unnamed = account_identity_event(local, Some(String::new()));
        assert_eq!(unnamed.peer_id, named.peer_id);
        assert_eq!(unnamed.title, None);

        let unavailable = account_identity_event(local, None);
        assert_eq!(unavailable.peer_id, unnamed.peer_id);
        assert_eq!(unavailable.title, None);
    }

    #[test]
    fn recognizes_only_groups_containing_the_local_aci() {
        let Some(ServiceId::Aci(local)) =
            ServiceId::parse_from_service_id_string("11111111-1111-4111-8111-111111111111")
        else {
            panic!("test ACI must parse");
        };
        let Some(ServiceId::Aci(other)) =
            ServiceId::parse_from_service_id_string("22222222-2222-4222-8222-222222222222")
        else {
            panic!("test ACI must parse");
        };

        assert!(contains_local_aci([&other, &local].into_iter(), &local));
        assert!(!contains_local_aci([&other].into_iter(), &local));
    }

    #[test]
    fn remembers_departed_groups_across_worker_clones() {
        let departed = DepartedGroups::default();
        let worker_copy = departed.clone();

        assert!(!worker_copy.contains("opaque-group-id"));
        departed.mark_departed("opaque-group-id".to_owned());
        assert!(worker_copy.contains("opaque-group-id"));
    }

    #[test]
    fn failed_leave_preserves_group_and_reports_its_identity() {
        let departed = DepartedGroups::default();
        departed.begin_leave("opaque-group-id".to_owned());
        assert!(departed.contains("opaque-group-id"));
        let events = group_leave_completion_events(
            &departed,
            41,
            "opaque-group-id",
            GroupLeaveCompletion::Failed("server rejected leave".to_owned()),
        );

        assert!(!departed.contains("opaque-group-id"));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, crate::event::EVENT_ERROR);
        assert_eq!(events[0].request_id, 41);
        assert_eq!(events[0].chat_id.as_deref(), Some("opaque-group-id"));
        assert_eq!(events[0].text.as_deref(), Some("server rejected leave"));
    }

    #[test]
    fn accepted_leave_is_terminal_before_success_is_reported() {
        let departed = DepartedGroups::default();
        departed.begin_leave("opaque-group-id".to_owned());
        let events = group_leave_completion_events(
            &departed,
            42,
            "opaque-group-id",
            GroupLeaveCompletion::Accepted {
                peer_notification_sent: true,
                local_group_removed: true,
            },
        );

        assert!(departed.contains("opaque-group-id"));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, EVENT_GROUP_LEFT);
        assert_eq!(events[0].request_id, 42);
        assert_eq!(events[0].chat_id.as_deref(), Some("opaque-group-id"));
    }

    #[test]
    fn leave_waits_for_an_in_flight_group_operation_before_departing() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let groups = DepartedGroups::default();
            let attachment_operation = groups.lock_operation().await;
            groups.begin_leave("opaque-group-id".to_owned());

            let leave_groups = groups.clone();
            let leave_entered = Arc::new(AtomicBool::new(false));
            let leave_entered_task = Arc::clone(&leave_entered);
            let leave = tokio::spawn(async move {
                let _leave_operation = leave_groups.lock_operation().await;
                leave_entered_task.store(true, Ordering::Release);
                leave_groups.mark_departed("opaque-group-id".to_owned());
            });

            tokio::task::yield_now().await;
            assert!(!leave_entered.load(Ordering::Acquire));
            drop(attachment_operation);
            leave.await.unwrap();
            assert!(leave_entered.load(Ordering::Acquire));
            assert!(groups.is_departed("opaque-group-id"));
        });
    }

    #[test]
    fn warns_only_for_incomplete_post_leave_cleanup() {
        assert!(group_leave_warning_messages(true, true).is_empty());
        assert_eq!(group_leave_warning_messages(false, true).len(), 1);
        assert_eq!(group_leave_warning_messages(true, false).len(), 1);
        assert_eq!(group_leave_warning_messages(false, false).len(), 2);
    }

    #[test]
    fn creates_a_png_qr_code() {
        let png = qr_png(b"sgnl://linkdevice?uuid=test&pub_key=test").unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(png.len() > 100);
    }

    #[test]
    fn detects_group_content_for_snapshot_refresh() {
        let direct = ContentBody::DataMessage(DataMessage::default());
        let group = ContentBody::DataMessage(DataMessage {
            group_v2: Some(GroupContextV2::default()),
            ..Default::default()
        });

        assert!(!content_has_group_context(&direct));
        assert!(content_has_group_context(&group));
        assert!(content_is_projectable(&direct, false));
        assert!(!content_is_projectable(&group, false));
        assert!(content_is_projectable(&group, true));
    }

    #[test]
    fn projection_dispositions_preserve_retryable_content() {
        assert_eq!(
            projection_effect(ProjectionDisposition::AwaitingAck),
            ProjectionEffect {
                remove_pending: false,
                mark_projected: false,
            }
        );
        assert_eq!(
            projection_effect(ProjectionDisposition::Complete),
            ProjectionEffect {
                remove_pending: true,
                mark_projected: true,
            }
        );
        assert_eq!(
            projection_effect(ProjectionDisposition::Retry),
            ProjectionEffect {
                remove_pending: true,
                mark_projected: false,
            }
        );
    }

    #[test]
    fn parses_direct_valid_and_malformed_group_contexts() {
        assert_eq!(
            group_message_target(&DataMessage::default()),
            GroupMessageTarget::Direct
        );
        assert_eq!(
            group_message_target(&DataMessage {
                group_v2: Some(GroupContextV2 {
                    master_key: Some(vec![7; 32]),
                    ..GroupContextV2::default()
                }),
                ..DataMessage::default()
            }),
            GroupMessageTarget::Group([7; 32])
        );
        assert_eq!(
            group_message_target(&DataMessage {
                group_v2: Some(GroupContextV2 {
                    master_key: Some(vec![7; 31]),
                    ..GroupContextV2::default()
                }),
                ..DataMessage::default()
            }),
            GroupMessageTarget::Malformed
        );
        assert_eq!(
            group_message_target(&DataMessage {
                group_v2: Some(GroupContextV2::default()),
                ..DataMessage::default()
            }),
            GroupMessageTarget::Malformed
        );
    }

    #[test]
    fn classifies_bare_messages_by_their_signal_author() {
        let Some(ServiceId::Aci(local)) =
            ServiceId::parse_from_service_id_string("11111111-1111-4111-8111-111111111111")
        else {
            panic!("local test ACI must parse");
        };
        let Some(ServiceId::Aci(remote)) =
            ServiceId::parse_from_service_id_string("22222222-2222-4222-8222-222222222222")
        else {
            panic!("remote test ACI must parse");
        };
        let local_id = ServiceId::Aci(local);
        let remote_id = ServiceId::Aci(remote);

        assert_eq!(
            bare_data_message_route(local_id, remote_id, local),
            BareDataMessageRoute {
                peer: remote_id.service_id_string(),
                outgoing: true,
            }
        );
        assert_eq!(
            bare_data_message_route(remote_id, local_id, local),
            BareDataMessageRoute {
                peer: remote_id.service_id_string(),
                outgoing: false,
            }
        );
    }

    #[test]
    fn keeps_the_local_author_as_the_outgoing_group_peer() {
        let Some(ServiceId::Aci(local)) =
            ServiceId::parse_from_service_id_string("11111111-1111-4111-8111-111111111111")
        else {
            panic!("local test ACI must parse");
        };
        let remote = "aci:22222222-2222-4222-8222-222222222222";

        assert_eq!(
            group_message_peer(true, remote, local),
            ServiceId::Aci(local).service_id_string()
        );
        assert_eq!(group_message_peer(false, remote, local), remote);
    }

    #[test]
    fn suppresses_only_inline_image_placeholders_from_projected_text() {
        assert_eq!(
            projected_data_message_text(String::new(), [(Some("photo.jpg"), true)]),
            None
        );
        assert_eq!(
            projected_data_message_text("caption".to_owned(), [(Some("photo.jpg"), true)]),
            Some("caption".to_owned())
        );
        assert_eq!(
            projected_data_message_text(String::new(), [(Some("photo.jpg"), false)]),
            Some("[Attachment: photo.jpg]".to_owned())
        );
        assert_eq!(
            projected_data_message_text(String::new(), [(Some("inline.png"), true), (None, false)]),
            Some("[Attachment: attachment]".to_owned())
        );
    }

    #[test]
    fn pending_leave_retries_projection_but_departure_completes_it() {
        let groups = DepartedGroups::default();
        groups.begin_leave("opaque-group-id".to_owned());
        assert_eq!(
            departure_projection_disposition(groups.departure_state("opaque-group-id")),
            Some(ProjectionDisposition::Retry)
        );

        groups.mark_departed("opaque-group-id".to_owned());
        assert_eq!(
            departure_projection_disposition(groups.departure_state("opaque-group-id")),
            Some(ProjectionDisposition::Complete)
        );
        assert_eq!(
            departure_projection_disposition(GroupDepartureState::Active),
            None
        );
    }

    #[test]
    fn bounds_and_resets_connection_recovery_backoff() {
        let mut backoff = RecoveryBackoff::default();

        let table_delays: Vec<u64> = (0..RECOVERY_RETRY_DELAYS_SECS.len())
            .map(|_| backoff.next_delay().as_secs())
            .collect();
        assert_eq!(table_delays, RECOVERY_RETRY_DELAYS_SECS);

        let longest = *RECOVERY_RETRY_DELAYS_SECS.last().unwrap();
        assert_eq!(backoff.next_delay(), Duration::from_secs(longest));
        assert_eq!(backoff.next_delay(), Duration::from_secs(longest));

        backoff.reset();
        assert_eq!(backoff.next_delay(), Duration::ZERO);
        assert_eq!(backoff.next_delay(), Duration::from_secs(1));
    }

    #[test]
    fn models_session_and_group_transitions_deterministically() {
        let mut session = SessionState::default();

        assert_eq!(session.phase, SessionPhase::Initializing);
        assert!(!session.is_ready());
        assert!(!session.is_recovering());
        assert!(!session.groups_authoritative());
        assert!(!session.groups_dirty());

        session.note_group_content(true);
        assert_eq!(session.groups, GroupSnapshotState::Pending);
        session.mark_groups_authoritative();
        assert!(session.groups_authoritative());
        assert!(!session.groups_dirty());
        session.note_group_content(true);
        assert_eq!(session.groups, GroupSnapshotState::Dirty);
        session.mark_groups_pending();
        assert_eq!(session.groups, GroupSnapshotState::Pending);

        session.mark_groups_authoritative();
        session.mark_ready();
        assert_eq!(session.phase, SessionPhase::Ready);
        assert!(session.is_ready());
        assert!(session.groups_authoritative());

        assert_eq!(
            session.enter_recovery("stream ended".to_owned()),
            RecoveryTransition::Entered
        );
        assert!(session.is_recovering());
        assert!(!session.is_ready());
        assert!(!session.groups_authoritative());
        assert_eq!(session.last_recovery_error(), Some("stream ended"));
        assert_eq!(session.next_recovery_delay(), Duration::ZERO);

        assert_eq!(
            session.enter_recovery("still unavailable".to_owned()),
            RecoveryTransition::Continued
        );
        assert_eq!(session.last_recovery_error(), Some("still unavailable"));
        assert_eq!(session.next_recovery_delay(), Duration::from_secs(1));

        session.mark_groups_authoritative();
        session.mark_ready();
        assert!(session.is_ready());
        assert_eq!(
            session.enter_recovery("stream ended again".to_owned()),
            RecoveryTransition::Entered
        );
        assert_eq!(session.next_recovery_delay(), Duration::ZERO);
    }

    #[test]
    fn models_fast_connect_decoupled_transitions() {
        let mut session = SessionState::default();

        assert_eq!(session.phase, SessionPhase::Initializing);
        assert!(!session.is_ready());
        assert!(!session.groups_authoritative());

        // Decoupled startup: session is marked ready before remote group synchronization
        session.mark_ready();
        assert_eq!(session.phase, SessionPhase::Ready);
        assert!(session.is_ready());
        assert!(!session.groups_authoritative());

        // Background group synchronization completes later
        session.mark_groups_authoritative();
        assert!(session.groups_authoritative());
        assert!(session.is_ready());

        // Network interruption moves session to recovery and resets group authority
        assert_eq!(
            session.enter_recovery("connection lost".to_owned()),
            RecoveryTransition::Entered
        );
        assert!(session.is_recovering());
        assert!(!session.is_ready());
        assert!(!session.groups_authoritative());

        // Fast reconnect: session is marked ready immediately upon socket connect
        session.mark_ready();
        assert!(session.is_ready());
        assert!(!session.groups_authoritative());

        // Remote group synchronization finishes again
        session.mark_groups_authoritative();
        assert!(session.groups_authoritative());
    }

    #[test]
    fn retries_only_transient_receive_start_failures() {
        let websocket_closing =
            presage::Error::<presage_store_sqlite::SqliteStoreError>::ServiceError(
                ServiceError::WsClosing {
                    reason: "test close",
                },
            );
        let rate_limited = presage::Error::<presage_store_sqlite::SqliteStoreError>::ServiceError(
            ServiceError::RateLimitExceeded { retry_after: None },
        );
        let unauthorized = presage::Error::<presage_store_sqlite::SqliteStoreError>::ServiceError(
            ServiceError::Unauthorized,
        );
        let websocket_unauthorized =
            presage::Error::<presage_store_sqlite::SqliteStoreError>::ServiceError(
                ServiceError::WsError(Box::new(reqwest_websocket::Error::Handshake(
                    reqwest_websocket::HandshakeError::UnexpectedStatusCode("401".parse().unwrap()),
                ))),
            );
        let websocket_unavailable =
            presage::Error::<presage_store_sqlite::SqliteStoreError>::ServiceError(
                ServiceError::WsError(Box::new(reqwest_websocket::Error::Handshake(
                    reqwest_websocket::HandshakeError::UnexpectedStatusCode("503".parse().unwrap()),
                ))),
            );
        let sender_websocket_closing =
            presage::Error::<presage_store_sqlite::SqliteStoreError>::MessageSenderError(Box::new(
                MessageSenderError::ServiceError(ServiceError::WsClosing {
                    reason: "test close while sending",
                }),
            ));
        let relink = presage::Error::<presage_store_sqlite::SqliteStoreError>::RelinkNecessary;

        assert!(receive_error_is_transient(&websocket_closing));
        assert!(receive_error_is_transient(&rate_limited));
        assert!(receive_error_is_transient(&websocket_unavailable));
        assert!(receive_error_is_transient(&sender_websocket_closing));
        assert_eq!(
            delivery_receipt_failure_action(&sender_websocket_closing),
            DeliveryReceiptFailureAction::Recover
        );
        assert!(!receive_error_is_transient(&unauthorized));
        assert!(!receive_error_is_transient(&websocket_unauthorized));
        assert!(!receive_error_is_transient(&relink));
    }

    #[test]
    fn retries_sqlite_store_and_protocol_pool_timeout_failures() {
        let pool_timeout_protocol = || {
            SignalProtocolError::InvalidState(
                "sqlite",
                "pool timed out while waiting for an open connection".into(),
            )
        };
        let sender_pool_timeout_service =
            presage::Error::<presage_store_sqlite::SqliteStoreError>::MessageSenderError(Box::new(
                MessageSenderError::ServiceError(ServiceError::SignalProtocolError(
                    pool_timeout_protocol(),
                )),
            ));
        let sender_pool_timeout_proto =
            presage::Error::<presage_store_sqlite::SqliteStoreError>::MessageSenderError(Box::new(
                MessageSenderError::ProtocolError(pool_timeout_protocol()),
            ));
        let store_pool_timeout =
            presage::Error::<presage_store_sqlite::SqliteStoreError>::ProtocolError(
                pool_timeout_protocol(),
            );
        let permanent_protocol =
            presage::Error::<presage_store_sqlite::SqliteStoreError>::ProtocolError(
                SignalProtocolError::InvalidProtobufEncoding,
            );

        assert!(receive_error_is_transient(&sender_pool_timeout_service));
        assert!(receive_error_is_transient(&sender_pool_timeout_proto));
        assert!(receive_error_is_transient(&store_pool_timeout));
        assert!(!receive_error_is_transient(&permanent_protocol));

        assert_eq!(
            delivery_receipt_failure_action(&sender_pool_timeout_service),
            DeliveryReceiptFailureAction::Retry
        );
        assert_eq!(
            delivery_receipt_failure_action(&sender_pool_timeout_proto),
            DeliveryReceiptFailureAction::Retry
        );
        assert_eq!(
            delivery_receipt_failure_action(&store_pool_timeout),
            DeliveryReceiptFailureAction::Retry
        );
        assert_eq!(
            delivery_receipt_failure_action(&permanent_protocol),
            DeliveryReceiptFailureAction::Discard
        );

        let sqlx_pool_timeout = presage::Error::<presage_store_sqlite::SqliteStoreError>::Store(
            presage_store_sqlite::SqliteStoreError::Db(sqlx::Error::PoolTimedOut),
        );
        assert!(receive_error_is_transient(&sqlx_pool_timeout));
        assert_eq!(
            delivery_receipt_failure_action(&sqlx_pool_timeout),
            DeliveryReceiptFailureAction::Retry
        );
    }

    #[test]
    fn keeps_link_preview_images_out_of_regular_attachments() {
        let preview_image = AttachmentPointer {
            file_name: Some("preview.jpg".into()),
            content_type: Some("image/jpeg".into()),
            ..AttachmentPointer::default()
        };
        let message = DataMessage {
            preview: vec![presage::proto::Preview {
                url: Some("https://example.invalid/article".into()),
                image: Some(preview_image),
                ..Default::default()
            }],
            ..DataMessage::default()
        };

        assert!(regular_message_attachments(&message).is_empty());
        assert_eq!(
            data_message_text(&message),
            "https://example.invalid/article"
        );

        let message = DataMessage {
            attachments: vec![AttachmentPointer {
                file_name: Some("actual.pdf".into()),
                ..AttachmentPointer::default()
            }],
            preview: message.preview,
            ..DataMessage::default()
        };
        assert_eq!(regular_message_attachments(&message).len(), 1);
        assert_eq!(
            regular_message_attachments(&message)[0]
                .file_name
                .as_deref(),
            Some("actual.pdf")
        );
    }

    #[test]
    fn sender_attachment_size_truncates_only_after_download() {
        let attachment = AttachmentPointer {
            size: Some(3),
            content_type: Some("application/octet-stream".into()),
            ..AttachmentPointer::default()
        };
        let download_pointer = attachment_pointer_without_sender_size_hint(&attachment);
        assert_eq!(download_pointer.size, None);
        assert_eq!(download_pointer.content_type, attachment.content_type);

        let mut padded = vec![1, 2, 3, 4, 5];
        truncate_attachment_to_sender_size(&attachment, &mut padded);
        assert_eq!(padded, vec![1, 2, 3]);

        let mut exact = vec![1, 2, 3];
        truncate_attachment_to_sender_size(&attachment, &mut exact);
        assert_eq!(exact, vec![1, 2, 3]);

        let larger = AttachmentPointer {
            size: Some(8),
            ..AttachmentPointer::default()
        };
        let mut shorter = vec![1, 2, 3];
        truncate_attachment_to_sender_size(&larger, &mut shorter);
        assert_eq!(shorter, vec![1, 2, 3]);

        let missing = AttachmentPointer::default();
        truncate_attachment_to_sender_size(&missing, &mut shorter);
        assert_eq!(shorter, vec![1, 2, 3]);

        let empty = AttachmentPointer {
            size: Some(0),
            ..AttachmentPointer::default()
        };
        truncate_attachment_to_sender_size(&empty, &mut shorter);
        assert!(shorter.is_empty());
    }

    #[test]
    fn metadata_cache_indexes_and_invalidates_groups_and_contacts() {
        let cache = MetadataCache::default();
        let key = [42u8; 32];
        let identifier = group_identifier(&key);
        assert_eq!(cache.group_key_for_identifier(&identifier), None);
        cache.index_group(identifier.clone(), key);
        assert_eq!(cache.group_key_for_identifier(&identifier), Some(key));

        cache.remove_group_index(&identifier);
        assert_eq!(cache.group_key_for_identifier(&identifier), None);

        let peer = "00000000-0000-0000-0000-000000000001";
        assert_eq!(cache.get_contact_name(peer), None);
        cache.put_contact_name(peer.to_string(), "Alice".to_string());
        assert_eq!(cache.get_contact_name(peer), Some("Alice".to_string()));

        cache.invalidate_contact(peer);
        assert_eq!(cache.get_contact_name(peer), None);

        cache.index_group(identifier.clone(), key);
        cache.put_contact_name(peer.to_string(), "Bob".to_string());
        cache.clear();
        assert_eq!(cache.group_key_for_identifier(&identifier), None);
        assert_eq!(cache.get_contact_name(peer), None);
    }

    fn test_thread() -> Thread {
        Thread::Contact(parse_recipient("00000000-0000-0000-0000-000000000042").unwrap())
    }

    #[test]
    fn mark_sent_message_projected_retries_transient_failures_before_succeeding() {
        use crate::store::traits::fake::{FakeStorageRepository, StorageErrorKind};

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let repo = FakeStorageRepository::new();
            let thread = test_thread();
            repo.fail_next_projection_attempts(StorageErrorKind::Transient, 2);
            let sent = SentMessage {
                thread: thread.clone(),
                timestamp: 42,
            };

            let result = mark_sent_message_projected(&repo, &sent).await;

            assert!(result.is_ok());
            assert!(repo.is_projected(&thread, 42));
        });
    }

    #[test]
    fn mark_sent_message_projected_retries_a_not_yet_visible_row() {
        use crate::store::traits::fake::{FakeStorageRepository, StorageErrorKind};

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let repo = FakeStorageRepository::new();
            let thread = test_thread();
            repo.fail_next_projection_attempts(StorageErrorKind::NotFound, 1);
            let sent = SentMessage {
                thread: thread.clone(),
                timestamp: 7,
            };

            let result = mark_sent_message_projected(&repo, &sent).await;

            assert!(result.is_ok());
            assert!(repo.is_projected(&thread, 7));
        });
    }

    #[test]
    fn mark_sent_message_projected_gives_up_after_exhausting_its_retry_budget() {
        use crate::store::traits::fake::{FakeStorageRepository, StorageErrorKind};

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let repo = FakeStorageRepository::new();
            let sent = SentMessage {
                thread: test_thread(),
                timestamp: 99,
            };
            // One more failure than MARK_SENT_PROJECTED_RETRY_DELAYS_MS has entries.
            repo.fail_next_projection_attempts(StorageErrorKind::Transient, 4);

            let result = mark_sent_message_projected(&repo, &sent).await;

            assert!(result.is_err());
            assert!(!repo.is_projected(&test_thread(), 99));
        });
    }
}
