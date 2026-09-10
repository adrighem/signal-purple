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
use presage::libsignal_service::protocol::{Aci, ServiceId, SignalProtocolError};
use presage::libsignal_service::sender::{AttachmentSpec, MessageSenderError};
use presage::libsignal_service::zkgroup::profiles::ProfileKey;
use presage::model::groups::Group;
use presage::proto::{
    AttachmentPointer, EditMessage, SyncMessage, TypingMessage, receipt_message, typing_message,
};
use presage::store::Thread;
use presage::{Manager, manager::Registered};
use presage_store_sqlite::ClientOutboxKind;
use presage_store_sqlite::SqliteStore;
use qrcode::QrCode;
use qrcode::types::Color;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex as AsyncMutex, mpsc as tokio_mpsc, watch};
use zeroize::{Zeroize, Zeroizing};

use super::media::{
    AvatarCache, DownloadedAttachment, MAX_SIGNAL_GIF_TRANSCODES_PER_MESSAGE,
    attachment_display_name, should_inline_image, transcode_signal_gif_video,
};
use super::outbox::{enqueue_and_send, retry_outbox};
use super::projection::*;
use crate::attachment::{AttachmentControl, AttachmentPermit, MAX_ATTACHMENT_BYTES};
use crate::event::{
    EVENT_ACCOUNT, EVENT_ATTACHMENT, EVENT_ATTACHMENT_SENT, EVENT_AVATAR, EVENT_CONTACT,
    EVENT_CONTACT_SYNC_BEGIN, EVENT_CONTACT_SYNC_END, EVENT_GROUP, EVENT_GROUP_LEFT,
    EVENT_GROUP_MEMBER, EVENT_GROUP_MESSAGE, EVENT_GROUP_SYNC_BEGIN, EVENT_GROUP_SYNC_END,
    EVENT_IDENTITY_ACCEPTED, EVENT_IDENTITY_CHANGE, EVENT_MESSAGE, EVENT_RECEIPT,
    EVENT_SESSION_RESET, EVENT_TYPING, Event, FLAG_OUTGOING,
};
use crate::event_queue::EventSink;
use crate::store::StorageRepository;

pub(crate) const GROUP_SYNC_RETRY_SECS: u64 = 30;
pub(crate) const RECOVERY_RETRY_DELAYS_SECS: [u64; 6] = [0, 1, 2, 4, 8, 16];
pub(crate) const RECEIVE_EVENT_QUEUE_CAPACITY: usize = 16;
pub(crate) const SHUTDOWN_CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
pub(crate) const SNAPSHOT_YIELD_INTERVAL: usize = 64;

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

pub struct StorePassphrase {
    value: Zeroizing<String>,
    #[cfg(test)]
    drop_observer: Option<Arc<AtomicBool>>,
}

impl StorePassphrase {
    pub fn new(value: String) -> Self {
        Self {
            value: Zeroizing::new(value),
            #[cfg(test)]
            drop_observer: None,
        }
    }

    pub(crate) fn as_str(&self) -> &str {
        self.value.as_str()
    }

    #[cfg(test)]
    pub(crate) fn observe_drop(&mut self, observer: Arc<AtomicBool>) {
        self.drop_observer = Some(observer);
    }
}

impl Drop for StorePassphrase {
    fn drop(&mut self) {
        self.value.zeroize();
        #[cfg(test)]
        if let Some(observer) = &self.drop_observer {
            observer.store(true, Ordering::Release);
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct MetadataCache {
    group_revisions: Arc<Mutex<HashMap<[u8; 32], u32>>>,
    contact_names: Arc<Mutex<HashMap<String, String>>>,
}

#[allow(dead_code)]
impl MetadataCache {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn get_group_revision(&self, master_key: &[u8; 32]) -> Option<u32> {
        self.group_revisions.lock().ok()?.get(master_key).copied()
    }

    pub(crate) fn put_group_revision(&self, master_key: [u8; 32], revision: u32) {
        if let Ok(mut guard) = self.group_revisions.lock() {
            guard.insert(master_key, revision);
        }
    }

    pub(crate) fn invalidate_group(&self, master_key: &[u8; 32]) {
        if let Ok(mut guard) = self.group_revisions.lock() {
            guard.remove(master_key);
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

    pub(crate) fn invalidate_contact(&self, peer_id: &str) {
        if let Ok(mut guard) = self.contact_names.lock() {
            guard.remove(peer_id);
        }
    }

    pub(crate) fn clear(&self) {
        if let Ok(mut guard) = self.group_revisions.lock() {
            guard.clear();
        }
        if let Ok(mut guard) = self.contact_names.lock() {
            guard.clear();
        }
    }
}

pub(crate) use super::worker::Command;

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

    async fn lock_operation(&self) -> tokio::sync::MutexGuard<'_, ()> {
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
    fn next_delay(&mut self) -> Option<Duration> {
        let seconds = *RECOVERY_RETRY_DELAYS_SECS.get(self.next_delay)?;
        self.next_delay += 1;
        Some(Duration::from_secs(seconds))
    }

    fn reset(&mut self) {
        self.next_delay = 0;
    }

    fn has_remaining(&self) -> bool {
        self.next_delay < RECOVERY_RETRY_DELAYS_SECS.len()
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

    pub(crate) fn next_recovery_delay(&mut self) -> Option<Duration> {
        debug_assert!(self.is_recovering());
        self.recovery_backoff.next_delay()
    }

    pub(crate) fn recovery_has_remaining(&self) -> bool {
        self.recovery_backoff.has_remaining()
    }

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

fn sqlx_error_is_transient(db_error: &sqlx::Error) -> bool {
    match db_error {
        sqlx::Error::PoolTimedOut => true,
        sqlx::Error::Database(err) => {
            if let Some(code) = err.code() {
                // Extended result codes in SQLite:
                // Primary: 5 (SQLITE_BUSY), 6 (SQLITE_LOCKED)
                // Extended: 261 (BUSY_RECOVERY), 517 (LOCKED_SHAREDCACHE),
                //           773 (BUSY_SNAPSHOT), 1029 (LOCKED_VTAB), 1032 (BUSY_TIMEOUT)
                if code == "5"
                    || code == "6"
                    || code.starts_with("5_")
                    || code.starts_with("6_")
                    || code == "261"
                    || code == "517"
                    || code == "773"
                    || code == "1029"
                    || code == "1032"
                {
                    return true;
                }
            }
            let message = err.message();
            message.contains("pool timed out")
                || message.contains("timed out")
                || message.contains("locked")
                || message.contains("busy")
        }
        sqlx::Error::Io(_) => true,
        _ => {
            let message = db_error.to_string();
            message.contains("pool timed out")
                || message.contains("timed out")
                || message.contains("locked")
                || message.contains("busy")
        }
    }
}

fn signal_protocol_error_is_transient(error: &SignalProtocolError) -> bool {
    match error {
        SignalProtocolError::InvalidState(scope, message) => {
            (*scope == "sqlite" || *scope == "presage sqlite store error")
                && (message.contains("pool timed out")
                    || message.contains("timed out")
                    || message.contains("locked")
                    || message.contains("busy")
                    || message.contains("code: 5")
                    || message.contains("code: 6")
                    || message.contains("code: 1032"))
        }
        _ => false,
    }
}

pub(crate) fn sqlite_store_error_is_transient(
    error: &presage_store_sqlite::SqliteStoreError,
) -> bool {
    match error {
        presage_store_sqlite::SqliteStoreError::Db(db_error) => sqlx_error_is_transient(db_error),
        presage_store_sqlite::SqliteStoreError::Io(_) => true,
        presage_store_sqlite::SqliteStoreError::Protocol(error) => {
            signal_protocol_error_is_transient(error)
        }
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
    super::worker::run_after_start_signal(
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
                _ = super::worker::wait_for_shutdown(&mut shutdown) => return,
            }
        };
        match result {
            Ok(()) => return,
            Err(error) => {
                let error = format!("Could not request Signal contact synchronization: {error}");
                let Some(delay) = backoff.next_delay() else {
                    sink.emit(Event::transient_error(format!(
                        "{error}; automatic retries exhausted"
                    )));
                    return;
                };
                sink.emit(Event::transient_error(format!(
                    "{error}; retrying automatically"
                )));
                if !delay.is_zero() {
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = super::worker::wait_for_shutdown(&mut shutdown) => return,
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
    super::worker::run_after_start_signal(
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
                repo.contact_profile_key(&ServiceId::Aci(contact.uuid.into()))
                    .await
                    .ok()
                    .flatten()
            };

            if let Some(key) = profile_key {
                let is_cached = repo
                    .contact_avatar(contact.uuid, key)
                    .await
                    .ok()
                    .flatten()
                    .is_some();
                if !is_cached {
                    let fetch = manager.retrieve_profile_avatar_by_uuid(contact.uuid, key);
                    let result = tokio::select! {
                        res = fetch => res,
                        _ = super::worker::wait_for_shutdown(&mut shutdown) => return,
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
            metadata_cache.put_group_revision(key, group.revision);
            let is_cached = repo.group_avatar(key).await.ok().flatten().is_some();
            if !is_cached {
                let revision = metadata_cache
                    .get_group_revision(&key)
                    .unwrap_or(group.revision);
                let context = GroupContextV2 {
                    master_key: Some(key.to_vec()),
                    revision: Some(revision),
                    ..Default::default()
                };
                let fetch = manager.retrieve_group_avatar(context);
                let result = tokio::select! {
                    res = fetch => res,
                    _ = super::worker::wait_for_shutdown(&mut shutdown) => return,
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
        _ = super::worker::wait_for_shutdown(&mut shutdown) => {}
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
        mark_sent_message_projected_or_report(manager, &sent, sink).await;
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

pub(crate) async fn handle_command_interruptibly(
    manager: &mut Manager<SqliteStore, Registered>,
    command: Command,
    shutdown: &mut watch::Receiver<bool>,
    sink: &EventSink,
    departed_groups: &DepartedGroups,
    groups_authoritative: bool,
    timestamps: &MessageTimestampAllocator,
) -> bool {
    let mut operation = Box::pin(handle_command(
        manager,
        command,
        sink,
        departed_groups,
        groups_authoritative,
        timestamps,
    ));

    tokio::select! {
        () = &mut operation => false,
        _ = super::worker::wait_for_shutdown(shutdown) => true,
    }
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
                    repo.contact_profile_key(&ServiceId::Aci(contact.uuid.into()))
                        .await
                        .ok()
                        .flatten()
                };
                if let Some(key) = profile_key
                    && let Some(avatar) =
                        repo.contact_avatar(contact.uuid, key).await.ok().flatten()
                {
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
) -> Result<(), String> {
    let repo = StorageRepository::new(manager.store().clone());
    let groups = repo.groups().await?;

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
        metadata_cache.put_group_revision(key, group.revision);
        sink.emit(Event {
            kind: EVENT_GROUP,
            chat_id: Some(chat_id.clone()),
            title: Some(group.title),
            ..Event::default()
        });
        emitted_records += 1;
        if let Some(avatar) = repo.group_avatar(key).await.ok().flatten() {
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
    emit_group_snapshot(manager, sink, departed_groups, avatar_cache, metadata_cache).await
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

async fn mark_sent_message_projected(
    repo: &StorageRepository,
    sent: &SentMessage,
) -> Result<(), String> {
    repo.mark_sent_message_projected(&sent.thread, sent.timestamp)
        .await
}

pub(crate) async fn mark_sent_message_projected_or_report(
    manager: &Manager<SqliteStore, Registered>,
    sent: &SentMessage,
    sink: &EventSink,
) {
    let repo = StorageRepository::new(manager.store().clone());
    if let Err(error) = mark_sent_message_projected(&repo, sent).await {
        sink.emit(Event::error(error, false));
    }
}

#[derive(Debug)]
pub(crate) enum AttachmentPayload {
    Data(Vec<u8>),
    Path(std::path::PathBuf),
}

pub(crate) struct OutgoingAttachment {
    pub(crate) recipient: String,
    pub(crate) filename: String,
    pub(crate) content_type: String,
    pub(crate) data: AttachmentPayload,
    pub(crate) group: bool,
}

pub(crate) async fn upload_and_send_attachment(
    manager: &mut Manager<SqliteStore, Registered>,
    attachment: OutgoingAttachment,
    departed_groups: &DepartedGroups,
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
            resolve_active_group(manager, &recipient, departed_groups)
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
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    let timestamp = timestamps.next();
    if group {
        let (key, _) = group_target.expect("group target was resolved before upload");
        let _operation = departed_groups.lock_operation().await;
        let group = active_group_by_key(manager, key, departed_groups)
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
                },
                timestamp,
            )
            .await
            .map_err(|error| error.to_string())?;
        Ok(SentMessage {
            thread: Thread::Group(key),
            timestamp,
        })
    } else {
        let recipient = parse_recipient(&recipient)
            .ok_or_else(|| "Recipient is not a canonical Signal service identifier".to_owned())?;
        manager
            .send_message(
                recipient,
                DataMessage {
                    attachments: vec![pointer],
                    timestamp: Some(timestamp),
                    ..Default::default()
                },
                timestamp,
            )
            .await
            .map_err(|error| error.to_string())?;
        Ok(SentMessage {
            thread: Thread::Contact(recipient),
            timestamp,
        })
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

async fn handle_command(
    manager: &mut Manager<SqliteStore, Registered>,
    command: Command,
    sink: &EventSink,
    departed_groups: &DepartedGroups,
    groups_authoritative: bool,
    timestamps: &MessageTimestampAllocator,
) {
    if let Command::AcceptIdentity {
        request_id,
        recipient,
    } = command
    {
        let repo = StorageRepository::new(manager.store().clone());
        match repo.accept_identity_change(&recipient).await {
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
                retry_outbox(manager, sink, departed_groups, groups_authoritative).await;
            }
            Ok(false) => sink.emit(Event::request_error(
                request_id,
                "No verified identity change is pending for this contact",
            )),
            Err(error) => sink.emit(Event::request_error(
                request_id,
                format!("Could not accept the Signal identity change: {error}"),
            )),
        }
        return;
    }

    if let Command::DismissIdentity {
        request_id,
        recipient,
    } = command
    {
        let repo = StorageRepository::new(manager.store().clone());
        if let Err(error) = repo.dismiss_identity_change(&recipient).await {
            sink.emit(Event::request_error(
                request_id,
                format!("Could not dismiss the Signal identity notice: {error}"),
            ));
        }
        return;
    }

    if let Command::ResetSession {
        request_id,
        recipient,
    } = command
    {
        let service_id = match parse_recipient(&recipient) {
            Some(service_id) => service_id,
            None => {
                sink.emit(Event::request_error(
                    request_id,
                    "The recipient identifier could not be parsed as a Signal service ID",
                ));
                return;
            }
        };
        match manager.clear_sessions(&service_id).await {
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
        return;
    }

    if let Command::MarkRead {
        request_id,
        recipient,
        timestamp,
    } = command
    {
        let result = match parse_recipient(&recipient) {
            Some(recipient) => send_receipt(
                manager,
                recipient,
                timestamp,
                receipt_message::Type::Read,
                timestamps,
            )
            .await
            .map_err(|error| error.to_string()),
            None => Err("Recipient is not a canonical Signal service identifier".into()),
        };
        if let Err(error) = result {
            sink.emit(Event::transient_request_error(request_id, error));
        }
        return;
    }

    if let Command::LeaveGroup {
        request_id,
        group_key,
    } = command
    {
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
        let resolved = resolve_active_group_for_leave(manager, &group_key, departed_groups).await;
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

        match Box::pin(manager.leave_group(&key)).await {
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
                let repo = StorageRepository::new(manager.store().clone());
                if let Err(error) = repo.expedite_outbox_messages(&group_key).await {
                    sink.emit(Event::error(
                        format!("Could not schedule stale group messages for cleanup: {error}"),
                        false,
                    ));
                }
                retry_outbox(manager, sink, departed_groups, groups_authoritative).await;
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
        return;
    }

    let (request_id, result) = match command {
        Command::SendMessage {
            request_id,
            recipient,
            message,
        } => {
            let result = if parse_recipient(&recipient).is_some() {
                Box::pin(enqueue_and_send(
                    manager,
                    ClientOutboxKind::Direct,
                    recipient,
                    message,
                    departed_groups,
                    sink,
                    timestamps,
                ))
                .await
            } else {
                Err("Recipient is not a canonical Signal service identifier".into())
            };
            (request_id, result)
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
                match resolve_active_group(manager, &group_key, departed_groups).await {
                    Ok(Some(_)) => {
                        Box::pin(enqueue_and_send(
                            manager,
                            ClientOutboxKind::Group,
                            group_key,
                            message,
                            departed_groups,
                            sink,
                            timestamps,
                        ))
                        .await
                    }
                    Ok(None) => Err(
                        "Signal group is unavailable or this account is no longer a member".into(),
                    ),
                    Err(error) => Err(error),
                }
            };
            (request_id, result)
        }
        Command::SetTyping {
            request_id,
            recipient,
            typing,
        } => {
            let result = match parse_recipient(&recipient) {
                Some(recipient) => {
                    let timestamp = timestamps.next();
                    Box::pin(manager.send_message(
                        recipient,
                        TypingMessage {
                            timestamp: Some(timestamp),
                            action: Some(if typing {
                                typing_message::Action::Started.into()
                            } else {
                                typing_message::Action::Stopped.into()
                            }),
                            group_id: None,
                        },
                        timestamp,
                    ))
                    .await
                    .map_err(|error| error.to_string())
                }
                None => Err("Recipient is not a canonical Signal service identifier".into()),
            };
            (request_id, result)
        }
        Command::SendAttachment { .. } => unreachable!(),
        Command::LeaveGroup { .. } => unreachable!(),
        Command::AcceptIdentity { .. }
        | Command::DismissIdentity { .. }
        | Command::ResetSession { .. } => unreachable!(),
        Command::MarkRead { .. } => unreachable!(),
    };

    if let Err(error) = result {
        sink.emit(Event::transient_request_error(request_id, error));
    }
}

pub(crate) async fn handle_content(
    manager: &mut Manager<SqliteStore, Registered>,
    content: Content,
    delivery_id: u64,
    sink: &EventSink,
    departed_groups: &DepartedGroups,
    delivery_receipts: &mut DeliveryReceiptQueue,
    timestamps: &MessageTimestampAllocator,
) -> ProjectionDisposition {
    let timestamp = content_timestamp(&content);
    let sender = content.metadata.sender.service_id_string();
    let local_aci = manager.registration_data().service_ids.aci();

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
            let disposition = emit_data_message(manager, projection, sink, departed_groups).await;
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
            return emit_data_message(manager, projection, sink, departed_groups).await;
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

async fn emit_data_message(
    manager: &Manager<SqliteStore, Registered>,
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
        match group_for_projection(manager, group_key, departed_groups).await {
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
        for (attachment_index, attachment) in attachments.iter().enumerate() {
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
        let group_peer = group_message_peer(
            outgoing,
            peer,
            manager.registration_data().service_ids.aci(),
        );
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

async fn group_for_projection(
    manager: &Manager<SqliteStore, Registered>,
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

    let repo = StorageRepository::new(manager.store().clone());
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

    let local_aci = manager.registration_data().service_ids.aci();
    Ok(
        match group.filter(|group| group_contains_local_aci(group, &local_aci)) {
            Some(group) => ProjectionGroup::Active(group),
            None => ProjectionGroup::Complete,
        },
    )
}

async fn active_group_by_key(
    manager: &Manager<SqliteStore, Registered>,
    key: [u8; 32],
    departed_groups: &DepartedGroups,
) -> Result<Option<Group>, String> {
    if departed_groups.contains(&group_identifier(&key)) {
        return Ok(None);
    }
    let local_aci = manager.registration_data().service_ids.aci();
    let repo = StorageRepository::new(manager.store().clone());
    repo.active_group(key, &local_aci).await
}

pub(crate) async fn resolve_active_group(
    manager: &Manager<SqliteStore, Registered>,
    identifier: &str,
    departed_groups: &DepartedGroups,
) -> Result<Option<([u8; 32], Group)>, String> {
    if departed_groups.contains(identifier) {
        return Ok(None);
    }
    resolve_active_group_in_store(manager, identifier).await
}

async fn resolve_active_group_for_leave(
    manager: &Manager<SqliteStore, Registered>,
    identifier: &str,
    departed_groups: &DepartedGroups,
) -> Result<Option<([u8; 32], Group)>, String> {
    if departed_groups.is_departed(identifier) {
        return Ok(None);
    }
    resolve_active_group_in_store(manager, identifier).await
}

async fn resolve_active_group_in_store(
    manager: &Manager<SqliteStore, Registered>,
    identifier: &str,
) -> Result<Option<([u8; 32], Group)>, String> {
    let local_aci = manager.registration_data().service_ids.aci();
    let repo = StorageRepository::new(manager.store().clone());
    let groups = repo.groups().await?;
    Ok(groups.into_iter().find(|(key, group)| {
        group_identifier(key) == identifier && group_contains_local_aci(group, &local_aci)
    }))
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

        assert_eq!(
            std::iter::from_fn(|| backoff.next_delay())
                .map(|delay| delay.as_secs())
                .collect::<Vec<_>>(),
            RECOVERY_RETRY_DELAYS_SECS
        );
        assert!(!backoff.has_remaining());
        assert_eq!(backoff.next_delay(), None);

        backoff.reset();
        assert!(backoff.has_remaining());
        assert_eq!(backoff.next_delay(), Some(Duration::ZERO));
        assert_eq!(backoff.next_delay(), Some(Duration::from_secs(1)));
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
        assert_eq!(session.next_recovery_delay(), Some(Duration::ZERO));

        assert_eq!(
            session.enter_recovery("still unavailable".to_owned()),
            RecoveryTransition::Continued
        );
        assert_eq!(session.last_recovery_error(), Some("still unavailable"));
        assert_eq!(session.next_recovery_delay(), Some(Duration::from_secs(1)));

        session.mark_groups_authoritative();
        session.mark_ready();
        assert!(session.is_ready());
        assert_eq!(
            session.enter_recovery("stream ended again".to_owned()),
            RecoveryTransition::Entered
        );
        assert_eq!(session.next_recovery_delay(), Some(Duration::ZERO));
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
    fn metadata_cache_caches_and_invalidates_group_revisions_and_contacts() {
        let cache = MetadataCache::new();
        let key = [42u8; 32];
        assert_eq!(cache.get_group_revision(&key), None);
        cache.put_group_revision(key, 5);
        assert_eq!(cache.get_group_revision(&key), Some(5));

        cache.invalidate_group(&key);
        assert_eq!(cache.get_group_revision(&key), None);

        let peer = "00000000-0000-0000-0000-000000000001";
        assert_eq!(cache.get_contact_name(peer), None);
        cache.put_contact_name(peer.to_string(), "Alice".to_string());
        assert_eq!(cache.get_contact_name(peer), Some("Alice".to_string()));

        cache.invalidate_contact(peer);
        assert_eq!(cache.get_contact_name(peer), None);

        cache.put_group_revision(key, 12);
        cache.put_contact_name(peer.to_string(), "Bob".to_string());
        cache.clear();
        assert_eq!(cache.get_group_revision(&key), None);
        assert_eq!(cache.get_contact_name(peer), None);
    }
}
