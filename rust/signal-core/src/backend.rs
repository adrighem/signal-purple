// SPDX-License-Identifier: AGPL-3.0-only
use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures::{FutureExt, StreamExt, channel::oneshot, future::Abortable, pin_mut};
use presage::libsignal_service::configuration::SignalServers;
use presage::libsignal_service::content::{
    Content, ContentBody, DataMessage, GroupContextV2, ServiceError,
};
use presage::libsignal_service::groups_v2::Role;
use presage::libsignal_service::protocol::{Aci, ServiceId};
use presage::libsignal_service::sender::{AttachmentSpec, MessageSenderError};
use presage::model::groups::Group;
use presage::model::identity::OnNewIdentity;
use presage::model::messages::Received;
use presage::proto::{
    AttachmentPointer, EditMessage, ReceiptMessage, SyncMessage, TypingMessage, attachment_pointer,
    receipt_message, typing_message,
};
use presage::store::{ContentsStore, StateStore, Thread};
use presage::{Manager, manager::Registered};
use presage_store_sqlite::SqliteStore;
use presage_store_sqlite::{ClientOutboxKind, ClientOutboxMessage};
use qrcode::QrCode;
use qrcode::types::Color;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex as AsyncMutex, mpsc as tokio_mpsc, watch};
use zeroize::{Zeroize, Zeroizing};

use crate::acknowledgment::AcknowledgmentInbox;
#[cfg(test)]
use crate::attachment::AttachmentAdmission;
use crate::attachment::{AttachmentControl, AttachmentPermit, MAX_ATTACHMENT_BYTES};
use crate::event::{
    EVENT_ACCOUNT, EVENT_ATTACHMENT, EVENT_ATTACHMENT_SENT, EVENT_CONTACT,
    EVENT_CONTACT_SYNC_BEGIN, EVENT_CONTACT_SYNC_END, EVENT_DISCONNECTED, EVENT_GROUP,
    EVENT_GROUP_LEFT, EVENT_GROUP_MEMBER, EVENT_GROUP_MESSAGE, EVENT_GROUP_SYNC_BEGIN,
    EVENT_GROUP_SYNC_END, EVENT_IDENTITY_ACCEPTED, EVENT_IDENTITY_CHANGE, EVENT_LINK_QR,
    EVENT_MESSAGE, EVENT_READY, EVENT_RECEIPT, EVENT_RECOVERING, EVENT_TYPING, Event,
    FLAG_OUTGOING,
};
use crate::event_queue::EventSink;

const MESSAGE_PROJECTION_CLIENT: &str = "signal-purple-v1";
const MAX_INLINE_MEDIA_BYTES: usize = 8 * 1024 * 1024;
const MAX_INLINE_GIF_EDGE: usize = 8192;
const MAX_INLINE_GIF_PIXELS: usize = 16 * 1000 * 1000;
const MAX_INLINE_GIF_PIXEL_FRAMES: usize = 8 * 1000 * 1000;
const MAX_SIGNAL_GIF_TRANSCODES_PER_MESSAGE: usize = 2;
const SIGNAL_GIF_TRANSCODE_POLL_INTERVAL: Duration = Duration::from_millis(20);
const SIGNAL_GIF_TRANSCODE_TIMEOUT: Duration = Duration::from_secs(15);
const SIGNAL_GIF_FFMPEG: &str = "/usr/bin/ffmpeg";
const SIGNAL_GIF_PRLIMIT: &str = "/usr/bin/prlimit";
const SIGNAL_GIF_ADDRESS_SPACE_LIMIT: &str = "--as=1073741824:1073741824";
const GROUP_SYNC_RETRY_SECS: u64 = 30;
const RECOVERY_RETRY_DELAYS_SECS: [u64; 6] = [0, 1, 2, 4, 8, 16];
const RECEIVE_EVENT_QUEUE_CAPACITY: usize = 16;
const MAX_PENDING_MESSAGE_PROJECTIONS: usize = 64;
const MAX_PENDING_DELIVERY_RECEIPTS: usize = 4096;
const RECENT_PROJECTION_IDENTITY_LIMIT: usize = 4096;
const SHUTDOWN_CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
const SNAPSHOT_YIELD_INTERVAL: usize = 64;

static SIGNAL_GIF_TRANSCODE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Default)]
struct MessageTimestampAllocator {
    latest: Arc<AtomicU64>,
}

impl MessageTimestampAllocator {
    fn next(&self) -> u64 {
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

pub(crate) struct StorePassphrase {
    value: Zeroizing<String>,
    #[cfg(test)]
    drop_observer: Option<Arc<AtomicBool>>,
}

impl StorePassphrase {
    pub(crate) fn new(value: String) -> Self {
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

pub(crate) struct Config {
    pub(crate) store_path: String,
    pub(crate) device_name: String,
    pub(crate) passphrase: StorePassphrase,
}

pub(crate) struct WorkerContext {
    pub(crate) config: Config,
    pub(crate) commands: tokio_mpsc::Receiver<Command>,
    pub(crate) acknowledgments: Arc<AcknowledgmentInbox>,
    pub(crate) shutdown: watch::Receiver<bool>,
    pub(crate) events: EventSink,
    pub(crate) ready: Arc<AtomicBool>,
}

#[derive(Debug)]
pub enum Command {
    SendMessage {
        request_id: u64,
        recipient: String,
        message: String,
    },
    SendGroupMessage {
        request_id: u64,
        group_key: String,
        message: String,
    },
    LeaveGroup {
        request_id: u64,
        group_key: String,
    },
    SendAttachment {
        request_id: u64,
        recipient: String,
        filename: String,
        content_type: String,
        data: Vec<u8>,
        group: bool,
        permit: AttachmentPermit,
    },
    SetTyping {
        request_id: u64,
        recipient: String,
        typing: bool,
    },
    AcceptIdentity {
        request_id: u64,
        recipient: String,
    },
    DismissIdentity {
        request_id: u64,
        recipient: String,
    },
    MarkRead {
        request_id: u64,
        recipient: String,
        timestamp: u64,
    },
}

struct MessageProjection {
    next_delivery_id: u64,
    pending: HashMap<u64, Content>,
    identities: ProjectionIdentities,
    acknowledgments: Arc<AcknowledgmentInbox>,
    delivery_receipts: DeliveryReceiptQueue,
    delivery_receipt_tasks: tokio::task::JoinSet<DeliveryReceiptCompletion>,
}

#[derive(Default)]
struct MessageReplayQueue {
    ready: VecDeque<Content>,
    waiting_for_groups: VecDeque<Content>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ProjectionIdentity {
    sender: String,
    destination: String,
    timestamp_ms: i64,
}

#[derive(Default)]
struct ProjectionIdentities {
    pending: HashSet<ProjectionIdentity>,
    completed: HashSet<ProjectionIdentity>,
    completed_order: VecDeque<ProjectionIdentity>,
}

#[derive(Default)]
struct RecoveryBackoff {
    next_delay: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum SessionPhase {
    #[default]
    Initializing,
    Recovering,
    Ready,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum GroupSnapshotState {
    #[default]
    Pending,
    Authoritative,
    Dirty,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecoveryTransition {
    Entered,
    Continued,
}

#[derive(Default)]
struct SessionState {
    phase: SessionPhase,
    groups: GroupSnapshotState,
    recovery_backoff: RecoveryBackoff,
    last_recovery_error: Option<String>,
}

struct ReceiveStartError {
    message: String,
    transient: bool,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct DeliveryReceiptIdentity {
    recipient: String,
    message_timestamp: u64,
}

#[derive(Clone, Debug)]
struct PendingDeliveryReceipt {
    identity: DeliveryReceiptIdentity,
    recipient: ServiceId,
    send_timestamp: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum DeliveryReceiptState {
    #[default]
    Ready,
    InFlight,
    Deferred,
}

#[derive(Default)]
struct DeliveryReceiptQueue {
    pending: VecDeque<(PendingDeliveryReceipt, DeliveryReceiptState)>,
    identities: HashSet<DeliveryReceiptIdentity>,
    limit_reported: bool,
}

struct DeliveryReceiptCompletion {
    generation: u64,
    receipt: PendingDeliveryReceipt,
    result: Result<(), presage::Error<presage_store_sqlite::SqliteStoreError>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DeliveryReceiptFailureAction {
    Retry,
    Recover,
    Discard,
}

struct ActiveReceiveTasks {
    receive: tokio::task::JoinHandle<()>,
    contact_sync: tokio::task::JoinHandle<()>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProjectionDisposition {
    AwaitingAck,
    Complete,
    Retry,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProjectionEffect {
    remove_pending: bool,
    mark_projected: bool,
}

fn projection_effect(disposition: ProjectionDisposition) -> ProjectionEffect {
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

struct SentMessage {
    thread: Thread,
    timestamp: u64,
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

#[derive(Debug)]
struct OutboxAttemptError {
    message: String,
    retryable: bool,
}

#[derive(Clone, Default)]
struct DepartedGroups {
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

    fn contains(&self, identifier: &str) -> bool {
        self.departure_state(identifier) != GroupDepartureState::Active
    }

    fn is_departed(&self, identifier: &str) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .departed
            .contains(identifier)
    }

    fn begin_leave(&self, identifier: String) {
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

impl OutboxAttemptError {
    fn permanent(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: false,
        }
    }

    fn retryable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: true,
        }
    }

    fn should_retry(&self) -> bool {
        self.retryable
    }
}

impl std::fmt::Display for OutboxAttemptError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl MessageProjection {
    fn new(acknowledgments: Arc<AcknowledgmentInbox>) -> Self {
        Self {
            next_delivery_id: 0,
            pending: HashMap::new(),
            identities: ProjectionIdentities::default(),
            acknowledgments,
            delivery_receipts: DeliveryReceiptQueue::default(),
            delivery_receipt_tasks: tokio::task::JoinSet::new(),
        }
    }

    fn track(&mut self, content: Content) -> Option<u64> {
        if !self.has_capacity() {
            return None;
        }
        if !self.identities.reserve(projection_identity(&content)) {
            return None;
        }
        self.next_delivery_id = self.next_delivery_id.wrapping_add(1).max(1);
        let delivery_id = self.next_delivery_id;
        self.pending.insert(delivery_id, content);
        self.acknowledgments.register(delivery_id);
        Some(delivery_id)
    }

    fn has_capacity(&self) -> bool {
        self.pending.len() < MAX_PENDING_MESSAGE_PROJECTIONS
    }

    fn release(&mut self, delivery_id: u64) -> Option<Content> {
        let content = self.pending.remove(&delivery_id)?;
        self.acknowledgments.unregister(delivery_id);
        self.identities
            .release_pending(&projection_identity(&content));
        Some(content)
    }

    fn complete(&mut self, delivery_id: u64) -> Option<Content> {
        let content = self.pending.remove(&delivery_id)?;
        self.acknowledgments.unregister(delivery_id);
        self.identities.complete(projection_identity(&content));
        Some(content)
    }
}

impl MessageReplayQueue {
    fn replace(&mut self, messages: Vec<Content>, groups_authoritative: bool) {
        self.ready.clear();
        self.waiting_for_groups.clear();
        for content in messages {
            self.push(content, groups_authoritative);
        }
    }

    fn push(&mut self, content: Content, groups_authoritative: bool) {
        if content_has_group_context(&content.body) && !groups_authoritative {
            self.waiting_for_groups.push_back(content);
        } else {
            self.ready.push_back(content);
        }
    }

    fn activate_groups(&mut self) {
        self.ready.append(&mut self.waiting_for_groups);
    }

    fn pop_ready(&mut self) -> Option<Content> {
        self.ready.pop_front()
    }

    fn can_accept_live_message(&self) -> bool {
        self.ready
            .len()
            .saturating_add(self.waiting_for_groups.len())
            < MAX_PENDING_MESSAGE_PROJECTIONS
    }
}

impl DeliveryReceiptQueue {
    fn enqueue(
        &mut self,
        recipient: ServiceId,
        message_timestamp: u64,
        send_timestamp: u64,
    ) -> bool {
        let identity = DeliveryReceiptIdentity {
            recipient: recipient.service_id_string(),
            message_timestamp,
        };
        if self.identities.contains(&identity) {
            return false;
        }
        if self.pending.len() >= MAX_PENDING_DELIVERY_RECEIPTS {
            let should_report = !self.limit_reported;
            self.limit_reported = true;
            return should_report;
        }
        self.identities.insert(identity.clone());
        self.pending.push_back((
            PendingDeliveryReceipt {
                identity,
                recipient,
                send_timestamp,
            },
            DeliveryReceiptState::Ready,
        ));
        false
    }

    fn start_next(&mut self, session_ready: bool) -> Option<PendingDeliveryReceipt> {
        if !session_ready {
            return None;
        }
        let (receipt, state) = self
            .pending
            .iter_mut()
            .find(|(_, state)| *state == DeliveryReceiptState::Ready)?;
        *state = DeliveryReceiptState::InFlight;
        Some(receipt.clone())
    }

    fn complete(&mut self, identity: &DeliveryReceiptIdentity) {
        let Some(index) = self
            .pending
            .iter()
            .position(|(receipt, _)| &receipt.identity == identity)
        else {
            return;
        };
        let (receipt, _) = self
            .pending
            .remove(index)
            .expect("delivery receipt index came from the same queue");
        self.identities.remove(&receipt.identity);
        if self.pending.len() < MAX_PENDING_DELIVERY_RECEIPTS {
            self.limit_reported = false;
        }
    }

    fn retry(&mut self, identity: &DeliveryReceiptIdentity, immediately: bool) {
        if let Some((_, state)) = self
            .pending
            .iter_mut()
            .find(|(receipt, _)| &receipt.identity == identity)
        {
            *state = if immediately {
                DeliveryReceiptState::Ready
            } else {
                DeliveryReceiptState::Deferred
            };
        }
    }

    fn activate_retries(&mut self) {
        for (_, state) in &mut self.pending {
            if *state == DeliveryReceiptState::Deferred {
                *state = DeliveryReceiptState::Ready;
            }
        }
    }

    fn release_in_flight(&mut self) {
        for (_, state) in &mut self.pending {
            if *state == DeliveryReceiptState::InFlight {
                *state = DeliveryReceiptState::Ready;
            }
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.pending.len()
    }
}

impl ProjectionIdentities {
    fn reserve(&mut self, identity: ProjectionIdentity) -> bool {
        if self.completed.contains(&identity) {
            return false;
        }
        self.pending.insert(identity)
    }

    fn release_pending(&mut self, identity: &ProjectionIdentity) {
        self.pending.remove(identity);
    }

    fn complete(&mut self, identity: ProjectionIdentity) {
        self.pending.remove(&identity);
        if !self.completed.insert(identity.clone()) {
            return;
        }
        self.completed_order.push_back(identity);
        while self.completed_order.len() > RECENT_PROJECTION_IDENTITY_LIMIT {
            if let Some(expired) = self.completed_order.pop_front() {
                self.completed.remove(&expired);
            }
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
    fn is_recovering(&self) -> bool {
        self.phase == SessionPhase::Recovering
    }

    fn is_ready(&self) -> bool {
        self.phase == SessionPhase::Ready
    }

    fn groups_authoritative(&self) -> bool {
        matches!(
            self.groups,
            GroupSnapshotState::Authoritative | GroupSnapshotState::Dirty
        )
    }

    fn groups_dirty(&self) -> bool {
        self.groups == GroupSnapshotState::Dirty
    }

    fn note_group_content(&mut self, has_group_context: bool) {
        if has_group_context && self.groups == GroupSnapshotState::Authoritative {
            self.groups = GroupSnapshotState::Dirty;
        }
    }

    fn mark_groups_authoritative(&mut self) {
        self.groups = GroupSnapshotState::Authoritative;
    }

    fn mark_groups_pending(&mut self) {
        self.groups = GroupSnapshotState::Pending;
    }

    fn mark_ready(&mut self) {
        debug_assert!(!self.is_ready());
        self.phase = SessionPhase::Ready;
        self.recovery_backoff.reset();
    }

    fn enter_recovery(&mut self, error: String) -> RecoveryTransition {
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

    fn next_recovery_delay(&mut self) -> Option<Duration> {
        debug_assert!(self.is_recovering());
        self.recovery_backoff.next_delay()
    }

    fn recovery_has_remaining(&self) -> bool {
        self.recovery_backoff.has_remaining()
    }

    fn last_recovery_error(&self) -> Option<&str> {
        self.last_recovery_error.as_deref()
    }
}

fn projection_identity(content: &Content) -> ProjectionIdentity {
    ProjectionIdentity {
        sender: content.metadata.sender.service_id_string(),
        destination: content.metadata.destination.service_id_string(),
        timestamp_ms: content.metadata.timestamp.timestamp_millis(),
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
        _ => false,
    }
}

fn receive_error_is_transient(
    error: &presage::Error<presage_store_sqlite::SqliteStoreError>,
) -> bool {
    match error {
        presage::Error::IoError(_)
        | presage::Error::Timeout(_)
        | presage::Error::MessagePipeInterruptedError => true,
        presage::Error::ServiceError(error) => service_error_is_transient(error),
        presage::Error::MessageSenderError(error) => {
            matches!(error.as_ref(), MessageSenderError::ServiceError(error)
                if service_error_is_transient(error))
        }
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

fn delivery_receipt_failure_action(
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

pub(crate) fn run_worker(context: WorkerContext) {
    let WorkerContext {
        config,
        commands,
        acknowledgments,
        shutdown,
        events,
        ready,
    } = context;
    let sink = events;
    let worker_acknowledgments = Arc::clone(&acknowledgments);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?;
        let local = tokio::task::LocalSet::new();
        match run_local_future(
            runtime,
            local,
            run(
                config,
                commands,
                worker_acknowledgments,
                shutdown,
                sink.clone(),
                Arc::clone(&ready),
            ),
            SHUTDOWN_CLEANUP_TIMEOUT,
        ) {
            Ok(result) => result,
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }));

    acknowledgments.close();
    ready.store(false, Ordering::Release);
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => sink.emit(Event::error(error, true)),
        Err(_) => sink.emit(Event::error("The Signal backend panicked", true)),
    }
}

async fn run(
    config: Config,
    commands: tokio_mpsc::Receiver<Command>,
    acknowledgments: Arc<AcknowledgmentInbox>,
    mut shutdown: watch::Receiver<bool>,
    sink: EventSink,
    ready: Arc<AtomicBool>,
) -> Result<(), String> {
    let Config {
        store_path,
        device_name,
        passphrase,
    } = config;
    let Some(store) = open_encrypted_store(&store_path, passphrase, &mut shutdown).await? else {
        return Ok(());
    };
    drop(store_path);

    let Some(is_registered) = await_or_shutdown(store.is_registered(), &mut shutdown).await else {
        return Ok(());
    };
    let manager = if is_registered {
        let load = Manager::load_registered(store);
        pin_mut!(load);
        tokio::select! {
            result = &mut load => {
                result.map_err(|error| {
                    format!("Could not load linked Signal device: {error}")
                })?
            }
            _ = wait_for_shutdown(&mut shutdown) => return Ok(()),
        }
    } else {
        match link_device(store, &device_name, &mut shutdown, &sink).await? {
            Some(manager) => manager,
            None => return Ok(()),
        }
    };

    receive_and_command_loop(manager, commands, acknowledgments, shutdown, sink, ready).await
}

async fn open_encrypted_store(
    store_path: &str,
    passphrase: StorePassphrase,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<Option<SqliteStore>, String> {
    let result = {
        let open_store = SqliteStore::open_with_passphrase(
            store_path,
            Some(passphrase.as_str()),
            OnNewIdentity::TrustUnverified,
        );
        await_or_shutdown(open_store, shutdown).await
    };
    drop(passphrase);

    match result {
        Some(Ok(store)) => Ok(Some(store)),
        Some(Err(error)) => Err(format!("Could not open encrypted Signal store: {error}")),
        None => Ok(None),
    }
}

async fn wait_for_shutdown(shutdown: &mut watch::Receiver<bool>) {
    if *shutdown.borrow() {
        return;
    }
    let _ = shutdown.changed().await;
}

async fn await_or_shutdown<F>(future: F, shutdown: &mut watch::Receiver<bool>) -> Option<F::Output>
where
    F: Future,
{
    if *shutdown.borrow() {
        return None;
    }
    pin_mut!(future);
    tokio::select! {
        biased;
        _ = wait_for_shutdown(shutdown) => None,
        output = &mut future => Some(output),
    }
}

async fn finish_shutdown_cleanup<F>(future: F, timeout: Duration) -> bool
where
    F: Future<Output = ()>,
{
    tokio::time::timeout(timeout, future).await.is_ok()
}

fn shutdown_runtime(runtime: tokio::runtime::Runtime, timeout: Duration) {
    runtime.shutdown_timeout(timeout);
}

fn run_local_future<F>(
    runtime: tokio::runtime::Runtime,
    local: tokio::task::LocalSet,
    future: F,
    shutdown_timeout: Duration,
) -> std::thread::Result<F::Output>
where
    F: Future,
{
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        runtime.block_on(local.run_until(future))
    }));
    drop(local);
    shutdown_runtime(runtime, shutdown_timeout);
    result
}

pub(crate) fn ensure_store_parent(store_path: &str) -> Result<(), String> {
    let Some(parent) = Path::new(store_path).parent() else {
        return Ok(());
    };
    if !parent.exists() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create Signal store directory: {error}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
                .map_err(|error| format!("Could not secure Signal store directory: {error}"))?;
        }
    }
    Ok(())
}

async fn link_device(
    store: SqliteStore,
    device_name: &str,
    shutdown: &mut watch::Receiver<bool>,
    sink: &EventSink,
) -> Result<Option<Manager<SqliteStore, Registered>>, String> {
    let (link_tx, link_rx) = oneshot::channel();
    let link = Manager::link_secondary_device(
        store,
        SignalServers::Production,
        device_name.to_owned(),
        link_tx,
    );
    pin_mut!(link);

    let qr_sink = sink.clone();
    let qr = async move {
        if let Ok(url) = link_rx.await {
            let uri = url.to_string();
            match qr_png(uri.as_bytes()) {
                Ok(data) => qr_sink.emit(Event {
                    kind: EVENT_LINK_QR,
                    text: Some(uri),
                    data,
                    ..Event::default()
                }),
                Err(error) => qr_sink.emit(Event::error(
                    format!("Could not render the linking QR code: {error}"),
                    true,
                )),
            }
        }
    };
    pin_mut!(qr);
    let mut qr_finished = false;

    loop {
        tokio::select! {
            result = &mut link => {
                return result
                    .map(Some)
                    .map_err(|error| format!("Signal device linking failed: {error}"));
            }
            () = &mut qr, if !qr_finished => {
                qr_finished = true;
            }
            _ = wait_for_shutdown(shutdown) => return Ok(None),
        }
    }
}

async fn receive_and_command_loop(
    mut manager: Manager<SqliteStore, Registered>,
    mut commands: tokio_mpsc::Receiver<Command>,
    acknowledgments: Arc<AcknowledgmentInbox>,
    mut shutdown: watch::Receiver<bool>,
    sink: EventSink,
    ready: Arc<AtomicBool>,
) -> Result<(), String> {
    let Some(projection_initialization) = await_or_shutdown(
        manager
            .store()
            .initialize_message_projection(MESSAGE_PROJECTION_CLIENT),
        &mut shutdown,
    )
    .await
    else {
        return Ok(());
    };
    projection_initialization
        .map_err(|error| format!("Could not initialize durable message replay: {error}"))?;
    let Some(identity_initialization) = await_or_shutdown(
        manager.store().initialize_identity_change_tracking(),
        &mut shutdown,
    )
    .await
    else {
        return Ok(());
    };
    identity_initialization
        .map_err(|error| format!("Could not initialize identity-change tracking: {error}"))?;
    let Some(outbox_initialization) =
        await_or_shutdown(manager.store().initialize_client_outbox(), &mut shutdown).await
    else {
        return Ok(());
    };
    outbox_initialization
        .map_err(|error| format!("Could not initialize the encrypted outbox: {error}"))?;
    let timestamps = MessageTimestampAllocator::default();
    let mut projection = MessageProjection::new(Arc::clone(&acknowledgments));
    let mut replay = MessageReplayQueue::default();
    let mut deferred_commands = VecDeque::new();
    let mut attachment_tasks = tokio::task::JoinSet::new();
    let mut attachment_aborts = HashMap::new();
    let mut receive_generation = 0u64;
    let departed_groups = DepartedGroups::default();
    let mut retry_tick = tokio::time::interval(std::time::Duration::from_secs(5));
    retry_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut acknowledgment_retry_tick = tokio::time::interval(std::time::Duration::from_secs(5));
    acknowledgment_retry_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    acknowledgment_retry_tick.reset();
    let mut group_sync_retry_tick =
        tokio::time::interval(std::time::Duration::from_secs(GROUP_SYNC_RETRY_SECS));
    group_sync_retry_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    group_sync_retry_tick.reset();
    let mut session = SessionState::default();

    macro_rules! await_recovery_phase_or_stop {
        ($phase:expr) => {
            match await_or_shutdown($phase, &mut shutdown).await {
                Some(output) => output,
                None => {
                    stop_attachments_and_drain_acknowledgments(
                        &manager,
                        &sink,
                        &mut attachment_tasks,
                        &mut attachment_aborts,
                        &acknowledgments,
                        &mut projection,
                    )
                    .await;
                    return Ok(());
                }
            }
        };
    }

    loop {
        if session.is_recovering() {
            if drain_recovery_commands(&mut commands, &mut deferred_commands) {
                stop_attachments_and_drain_acknowledgments(
                    &manager,
                    &sink,
                    &mut attachment_tasks,
                    &mut attachment_aborts,
                    &acknowledgments,
                    &mut projection,
                )
                .await;
                return Ok(());
            }
            let Some(delay) = session.next_recovery_delay() else {
                let error = session
                    .last_recovery_error()
                    .unwrap_or("Signal message reception did not recover")
                    .to_owned();
                fail_deferred_commands(
                    &sink,
                    &mut deferred_commands,
                    "Signal connection recovery was exhausted before the request could be sent",
                );
                sink.emit(Event {
                    kind: EVENT_DISCONNECTED,
                    text: Some(error),
                    ..Event::default()
                });
                stop_attachments_and_drain_acknowledgments(
                    &manager,
                    &sink,
                    &mut attachment_tasks,
                    &mut attachment_aborts,
                    &acknowledgments,
                    &mut projection,
                )
                .await;
                return Ok(());
            };
            if !delay.is_zero() {
                let sleep = tokio::time::sleep(delay);
                pin_mut!(sleep);
                loop {
                    tokio::select! {
                        _ = &mut sleep => break,
                        command = commands.recv() => {
                            let Some(command) = command else {
                                stop_attachments_and_drain_acknowledgments(
                                    &manager,
                                    &sink,
                                    &mut attachment_tasks,
                                    &mut attachment_aborts,
                                    &acknowledgments,
                                    &mut projection,
                                ).await;
                                return Ok(());
                            };
                            handle_recovery_command(command, &mut deferred_commands);
                        }
                        _ = acknowledgments.wait() => {
                            await_recovery_phase_or_stop!(process_acknowledgments(
                                &manager,
                                &acknowledgments,
                                &sink,
                                &mut projection,
                                true,
                            ));
                        }
                        _ = acknowledgment_retry_tick.tick() => {
                            acknowledgments.activate_retries();
                        }
                        completed = attachment_tasks.join_next(),
                            if !attachment_tasks.is_empty() =>
                        {
                            if let Some(completed) = completed {
                                await_recovery_phase_or_stop!(handle_attachment_completion(
                                    &manager,
                                    &sink,
                                    &mut attachment_aborts,
                                    completed,
                                ));
                            }
                        }
                        _ = wait_for_shutdown(&mut shutdown) => {
                            stop_attachments_and_drain_acknowledgments(
                                &manager,
                                &sink,
                                &mut attachment_tasks,
                                &mut attachment_aborts,
                                &acknowledgments,
                                &mut projection,
                            ).await;
                            return Ok(());
                        },
                    }
                }
            }
        }

        // Keep Presage's stream scheduled independently. Its retained unfold
        // future can own the sole SQLite connection while pending, so parking
        // it while the actor awaits other store work would self-starve.
        let (receive_started, mut messages, mut receive_task) =
            spawn_receive_driver(manager.clone());
        let mut receive_started = Box::pin(receive_started);

        macro_rules! await_receive_start_phase_or_stop {
            ($phase:expr) => {
                match await_or_shutdown($phase, &mut shutdown).await {
                    Some(output) => output,
                    None => {
                        stop_receive_driver(&mut receive_task).await;
                        stop_attachments_and_drain_acknowledgments(
                            &manager,
                            &sink,
                            &mut attachment_tasks,
                            &mut attachment_aborts,
                            &acknowledgments,
                            &mut projection,
                        )
                        .await;
                        return Ok(());
                    }
                }
            };
        }

        let receive_started = loop {
            tokio::select! {
                result = &mut receive_started => {
                    break result.unwrap_or_else(|_| {
                        Err(ReceiveStartError {
                            message: "Signal message reception stopped during startup".to_owned(),
                            transient: true,
                        })
                    });
                }
                command = commands.recv(), if session.is_recovering() => {
                    let Some(command) = command else {
                        stop_receive_driver(&mut receive_task).await;
                        stop_attachments_and_drain_acknowledgments(
                            &manager,
                            &sink,
                            &mut attachment_tasks,
                            &mut attachment_aborts,
                            &acknowledgments,
                            &mut projection,
                        ).await;
                        return Ok(());
                    };
                    handle_recovery_command(command, &mut deferred_commands);
                }
                _ = acknowledgments.wait() => {
                    await_receive_start_phase_or_stop!(process_acknowledgments(
                        &manager,
                        &acknowledgments,
                        &sink,
                        &mut projection,
                        true,
                    ));
                }
                _ = acknowledgment_retry_tick.tick() => {
                    acknowledgments.activate_retries();
                }
                completed = attachment_tasks.join_next(),
                    if session.is_recovering() && !attachment_tasks.is_empty() =>
                {
                    if let Some(completed) = completed {
                        await_receive_start_phase_or_stop!(handle_attachment_completion(
                            &manager,
                            &sink,
                            &mut attachment_aborts,
                            completed,
                        ));
                    }
                }
                _ = wait_for_shutdown(&mut shutdown) => {
                    stop_receive_driver(&mut receive_task).await;
                    stop_attachments_and_drain_acknowledgments(
                        &manager,
                        &sink,
                        &mut attachment_tasks,
                        &mut attachment_aborts,
                        &acknowledgments,
                        &mut projection,
                    ).await;
                    return Ok(());
                },
            }
        };
        if let Err(ReceiveStartError { message, transient }) = receive_started {
            stop_receive_driver(&mut receive_task).await;
            let error = message;
            ready.store(false, Ordering::Release);
            if !transient {
                fail_deferred_commands(
                    &sink,
                    &mut deferred_commands,
                    "Signal connection recovery stopped before the request could be sent",
                );
                stop_attachments_and_drain_acknowledgments(
                    &manager,
                    &sink,
                    &mut attachment_tasks,
                    &mut attachment_aborts,
                    &acknowledgments,
                    &mut projection,
                )
                .await;
                return Err(error);
            }
            let transition = session.enter_recovery(error.clone());
            if transition == RecoveryTransition::Entered {
                sink.emit(Event {
                    kind: EVENT_RECOVERING,
                    ..Event::default()
                });
            }
            let status = if session.recovery_has_remaining() {
                "retrying automatically"
            } else {
                "automatic retries exhausted"
            };
            sink.emit(Event::transient_error(format!("{error}; {status}")));
            continue;
        }
        receive_generation = receive_generation.wrapping_add(1).max(1);

        // Keep the contact request completely unpolled until the first
        // QueueEmpty path has finished its startup store work.
        let (contact_sync_start, contact_sync_wait) = oneshot::channel();
        let mut contact_sync_start = Some(contact_sync_start);
        let contact_sync = tokio::task::spawn_local(request_contacts_after_queue_drain(
            contact_sync_wait,
            manager.clone(),
            shutdown.clone(),
            sink.clone(),
        ));
        let mut receive_tasks = ActiveReceiveTasks {
            receive: receive_task,
            contact_sync,
        };

        macro_rules! await_phase_or_stop {
            ($phase:expr) => {
                match await_or_shutdown($phase, &mut shutdown).await {
                    Some(output) => output,
                    None => {
                        stop_active_receive_loop(
                            &mut receive_tasks,
                            &manager,
                            &sink,
                            &mut attachment_tasks,
                            &mut attachment_aborts,
                            &acknowledgments,
                            &mut projection,
                        )
                        .await;
                        return Ok(());
                    }
                }
            };
        }

        let recovery_error = loop {
            let session_ready = session.is_ready();
            let groups_authoritative = session.groups_authoritative();
            let groups_dirty = session.groups_dirty();
            if groups_authoritative {
                replay.activate_groups();
            }
            if projection.has_capacity()
                && let Some(content) = replay.pop_ready()
            {
                await_phase_or_stop!(project_content(
                    &mut manager,
                    content,
                    &sink,
                    &mut projection,
                    &departed_groups,
                    groups_authoritative,
                    &timestamps,
                ));
                continue;
            }
            if projection.delivery_receipt_tasks.is_empty()
                && let Some(receipt) = projection.delivery_receipts.start_next(session_ready)
            {
                spawn_delivery_receipt_attempt(
                    &mut projection.delivery_receipt_tasks,
                    manager.clone(),
                    receive_generation,
                    receipt,
                );
            }
            tokio::select! {
                _ = acknowledgments.wait() => {
                    await_phase_or_stop!(process_acknowledgments(
                        &manager,
                        &acknowledgments,
                        &sink,
                        &mut projection,
                        true,
                    ));
                }
                received = messages.recv(), if projection.has_capacity()
                    && replay.can_accept_live_message() => {
                    match received {
                        Some(Received::QueueEmpty) => {
                            if !session_ready {
                                await_phase_or_stop!(
                                    emit_account_identity(&mut manager, &sink)
                                );
                                await_phase_or_stop!(emit_contact_snapshot(&manager, &sink));
                                match await_phase_or_stop!(
                                    synchronize_and_emit_group_snapshot(
                                        &mut manager,
                                        &sink,
                                        &departed_groups,
                                    )
                                ) {
                                    Ok(()) => session.mark_groups_authoritative(),
                                    Err(error) => {
                                        session.mark_groups_pending();
                                        sink.emit(Event::transient_error(error));
                                        group_sync_retry_tick.reset();
                                    }
                                }
                                await_phase_or_stop!(load_unprojected_messages(
                                    &manager,
                                    &sink,
                                    &mut replay,
                                    session.groups_authoritative(),
                                ));
                                await_phase_or_stop!(emit_identity_changes(&manager, &sink));
                                await_phase_or_stop!(retry_outbox(
                                    &mut manager,
                                    &sink,
                                    &departed_groups,
                                    session.groups_authoritative(),
                                ));
                                session.mark_ready();
                                projection.delivery_receipts.activate_retries();
                                ready.store(true, Ordering::Release);
                                sink.emit(Event { kind: EVENT_READY, ..Event::default() });
                                if let Some(start) = contact_sync_start.take() {
                                    let _ = start.send(());
                                }
                            } else if groups_dirty {
                                match await_phase_or_stop!(emit_group_snapshot(
                                    &manager,
                                    &sink,
                                    &departed_groups,
                                )) {
                                    Ok(()) => session.mark_groups_authoritative(),
                                    Err(error) => {
                                        session.mark_groups_pending();
                                        group_sync_retry_tick.reset();
                                        sink.emit(Event::transient_error(error));
                                    }
                                }
                            }
                        }
                        Some(Received::Contacts) => {
                            await_phase_or_stop!(emit_contact_snapshot(&manager, &sink));
                        }
                        Some(Received::Content(content)) => {
                            session.note_group_content(content_has_group_context(&content.body));
                            if session_ready {
                                replay.push(*content, groups_authoritative);
                                await_phase_or_stop!(emit_identity_changes(&manager, &sink));
                            }
                        }
                        None => break "Signal's message stream ended unexpectedly".to_owned(),
                    }
                }
                command = async {
                    if session_ready
                        && let Some(command) = deferred_commands.pop_front()
                    {
                        Some(command)
                    } else {
                        commands.recv().await
                    }
                } => {
                    let Some(command) = command else {
                        stop_active_receive_loop(
                            &mut receive_tasks,
                            &manager,
                            &sink,
                            &mut attachment_tasks,
                            &mut attachment_aborts,
                            &acknowledgments,
                            &mut projection,
                        ).await;
                        return Ok(());
                    };
                    if !session_ready {
                        handle_recovery_command(command, &mut deferred_commands);
                        continue;
                    }
                    match command {
                        Command::SendAttachment {
                            request_id,
                            recipient,
                            filename,
                            content_type,
                            data,
                            group,
                            permit,
                        } => {
                            if permit.is_cancelled() {
                                continue;
                            }
                            if group && !groups_authoritative {
                                if permit.claim_terminal() {
                                    sink.emit(Event::request_error(
                                        request_id,
                                        "Signal groups are temporarily unavailable until authoritative synchronization succeeds",
                                    ));
                                }
                                continue;
                            }
                            let mut attachment_manager = manager.clone();
                            let attachment_departed_groups = departed_groups.clone();
                            let attachment_timestamps = timestamps.clone();
                            let control = permit.control();
                            let task = attachment_tasks.spawn_local(async move {
                                attachment_task_result(
                                    request_id,
                                    permit,
                                    upload_and_send_attachment(
                                        &mut attachment_manager,
                                        OutgoingAttachment {
                                            recipient,
                                            filename,
                                            content_type,
                                            data,
                                            group,
                                        },
                                        &attachment_departed_groups,
                                        &attachment_timestamps,
                                    ),
                                )
                                .await
                            });
                            attachment_aborts.insert(
                                request_id,
                                AttachmentTaskControl { task, control },
                            );
                        }
                        command => {
                            if groups_authoritative
                                && let Command::LeaveGroup { group_key, .. } = &command
                            {
                                departed_groups.begin_leave(group_key.clone());
                            }
                            if handle_command_interruptibly(
                                &mut manager,
                                command,
                                &mut shutdown,
                                &sink,
                                &departed_groups,
                                groups_authoritative,
                                &timestamps,
                            ).await {
                                stop_active_receive_loop(
                                    &mut receive_tasks,
                                    &manager,
                                    &sink,
                                    &mut attachment_tasks,
                                    &mut attachment_aborts,
                                    &acknowledgments,
                                    &mut projection,
                                ).await;
                                return Ok(());
                            }
                            await_phase_or_stop!(emit_identity_changes(&manager, &sink));
                        }
                    }
                }
                completed = attachment_tasks.join_next(), if !attachment_tasks.is_empty() => {
                    if let Some(completed) = completed {
                        await_phase_or_stop!(handle_attachment_completion(
                            &manager,
                            &sink,
                            &mut attachment_aborts,
                            completed,
                        ));
                    }
                }
                completed = projection.delivery_receipt_tasks.join_next(),
                    if !projection.delivery_receipt_tasks.is_empty() =>
                {
                    let Some(completed) = completed else {
                        unreachable!("a non-empty delivery receipt task set returned no task")
                    };
                    match completed {
                        Ok(completion) => {
                            let generation = completion.generation;
                            if let Some(error) = finish_delivery_receipt_attempt(
                                &mut projection.delivery_receipts,
                                &sink,
                                &completion,
                            ) && generation == receive_generation {
                                break error;
                            }
                        }
                        Err(error) => {
                            projection.delivery_receipts.release_in_flight();
                            sink.emit(Event::error(
                                format!(
                                    "Signal delivery receipt worker stopped unexpectedly: {error}"
                                ),
                                false,
                            ));
                        }
                    }
                }
                _ = retry_tick.tick(), if session_ready => {
                    projection.delivery_receipts.activate_retries();
                    await_phase_or_stop!(retry_outbox(
                        &mut manager,
                        &sink,
                        &departed_groups,
                        groups_authoritative,
                    ));
                }
                _ = acknowledgment_retry_tick.tick() => {
                    acknowledgments.activate_retries();
                }
                _ = group_sync_retry_tick.tick(), if session_ready && !groups_authoritative => {
                    match await_phase_or_stop!(synchronize_and_emit_group_snapshot(
                        &mut manager,
                        &sink,
                        &departed_groups,
                    )) {
                        Ok(()) => {
                            session.mark_groups_authoritative();
                            replay.activate_groups();
                            await_phase_or_stop!(retry_outbox(
                                &mut manager,
                                &sink,
                                &departed_groups,
                                true,
                            ));
                        }
                        Err(error) => {
                            session.mark_groups_pending();
                            sink.emit(Event::transient_error(error));
                        }
                    }
                }
                _ = wait_for_shutdown(&mut shutdown) => {
                    stop_active_receive_loop(
                        &mut receive_tasks,
                        &manager,
                        &sink,
                        &mut attachment_tasks,
                        &mut attachment_aborts,
                        &acknowledgments,
                        &mut projection,
                    ).await;
                    return Ok(());
                },
            }
        };

        ready.store(false, Ordering::Release);
        let error = recovery_error;
        let transition = session.enter_recovery(error.clone());
        if transition == RecoveryTransition::Entered {
            sink.emit(Event {
                kind: EVENT_RECOVERING,
                ..Event::default()
            });
        }
        stop_receive_tasks(
            &mut receive_tasks,
            &manager,
            &sink,
            &mut attachment_tasks,
            &mut attachment_aborts,
            std::future::ready(()),
        )
        .await;
        sink.emit(Event::transient_error(format!(
            "{error}; reconnecting automatically"
        )));
    }
}

fn spawn_receive_driver(
    mut manager: Manager<SqliteStore, Registered>,
) -> (
    oneshot::Receiver<Result<(), ReceiveStartError>>,
    tokio_mpsc::Receiver<Received>,
    tokio::task::JoinHandle<()>,
) {
    let (started_tx, started_rx) = oneshot::channel();
    let (messages_tx, messages_rx) = tokio_mpsc::channel(RECEIVE_EVENT_QUEUE_CAPACITY);
    let task = tokio::task::spawn_local(async move {
        let messages = match manager.receive_messages().await {
            Ok(messages) => messages,
            Err(error) => {
                let transient = receive_error_is_transient(&error);
                let _ = started_tx.send(Err(ReceiveStartError {
                    message: format!("Could not start Signal message reception: {error}"),
                    transient,
                }));
                return;
            }
        };
        if started_tx.send(Ok(())).is_err() {
            return;
        }
        forward_stream_to_channel(messages, messages_tx).await;
    });
    (started_rx, messages_rx, task)
}

async fn forward_stream_to_channel<S, T>(stream: S, sender: tokio_mpsc::Sender<T>)
where
    S: futures::Stream<Item = T>,
{
    pin_mut!(stream);
    while let Some(item) = stream.next().await {
        if sender.send(item).await.is_err() {
            return;
        }
    }
}

async fn stop_receive_driver(task: &mut tokio::task::JoinHandle<()>) {
    task.abort();
    let _ = task.await;
}

async fn run_after_start_signal<F>(start: oneshot::Receiver<()>, operation: F)
where
    F: Future<Output = ()>,
{
    if start.await.is_ok() {
        operation.await;
    }
}

async fn request_contacts_after_queue_drain(
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
                        _ = wait_for_shutdown(&mut shutdown) => return,
                    }
                }
            }
        }
    }
}

async fn handle_attachment_completion(
    manager: &Manager<SqliteStore, Registered>,
    sink: &EventSink,
    attachment_aborts: &mut HashMap<u64, AttachmentTaskControl>,
    completed: Result<AttachmentCompletion, tokio::task::JoinError>,
) {
    if let Some(sent) = finish_attachment_completion(sink, attachment_aborts, completed) {
        mark_sent_message_projected_or_report(manager, &sent, sink).await;
    }
}

fn finish_attachment_completion(
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

enum AttachmentTaskResult {
    Finished(Result<SentMessage, String>),
    Cancelled,
}

struct AttachmentTaskControl {
    task: tokio::task::AbortHandle,
    control: AttachmentControl,
}

struct AttachmentCompletion {
    request_id: u64,
    result: AttachmentTaskResult,
    permit: AttachmentPermit,
}

async fn attachment_task_result(
    request_id: u64,
    mut permit: AttachmentPermit,
    task: impl Future<Output = Result<SentMessage, String>>,
) -> AttachmentCompletion {
    let cancellation = permit.take_cancellation_registration();
    let task = std::panic::AssertUnwindSafe(task).catch_unwind();
    let result = match Abortable::new(task, cancellation).await {
        Ok(result) => {
            let result = result
                .unwrap_or_else(|_| Err("Signal attachment task failed unexpectedly".to_owned()));
            if permit.claim_terminal() || result.is_ok() {
                AttachmentTaskResult::Finished(result)
            } else {
                AttachmentTaskResult::Cancelled
            }
        }
        Err(_) => AttachmentTaskResult::Cancelled,
    };
    AttachmentCompletion {
        request_id,
        result,
        permit,
    }
}

async fn abort_in_flight_attachments(
    manager: &Manager<SqliteStore, Registered>,
    sink: &EventSink,
    attachment_tasks: &mut tokio::task::JoinSet<AttachmentCompletion>,
    attachment_aborts: &mut HashMap<u64, AttachmentTaskControl>,
) {
    let completions = abort_and_drain_tasks(
        attachment_tasks,
        attachment_aborts.values().map(|control| &control.task),
    )
    .await;
    let mut sent_messages = Vec::new();
    for completed in completions {
        if let Some(sent) = finish_attachment_completion(sink, attachment_aborts, completed) {
            sent_messages.push(sent);
        }
    }
    for sent in sent_messages {
        mark_sent_message_projected_or_report(manager, &sent, sink).await;
    }
    interrupt_remaining_attachments(sink, attachment_aborts);
}

fn interrupt_remaining_attachments(
    sink: &EventSink,
    attachment_controls: &mut HashMap<u64, AttachmentTaskControl>,
) {
    for (request_id, control) in attachment_controls.drain() {
        if let Some(event) = interrupted_attachment_event(request_id, &control.control) {
            sink.emit(event);
        }
    }
}

fn abandon_timed_out_attachments(
    sink: &EventSink,
    attachment_tasks: &mut tokio::task::JoinSet<AttachmentCompletion>,
    attachment_controls: &mut HashMap<u64, AttachmentTaskControl>,
) {
    let mut abandoned_tasks = std::mem::replace(attachment_tasks, tokio::task::JoinSet::new());
    abandoned_tasks.abort_all();
    drop(abandoned_tasks);
    interrupt_remaining_attachments(sink, attachment_controls);
}

fn interrupted_attachment_event(request_id: u64, control: &AttachmentControl) -> Option<Event> {
    if control.is_cancelled() {
        None
    } else {
        let _ = control.claim_terminal();
        Some(Event::request_error(
            request_id,
            "Signal connection was interrupted before the attachment completed",
        ))
    }
}

async fn abort_and_drain_tasks<T: Send + 'static>(
    tasks: &mut tokio::task::JoinSet<T>,
    aborts: impl Iterator<Item = &tokio::task::AbortHandle>,
) -> Vec<Result<T, tokio::task::JoinError>> {
    for abort in aborts {
        abort.abort();
    }
    tasks.abort_all();
    let mut completions = Vec::with_capacity(tasks.len());
    while let Some(completed) = tasks.join_next().await {
        completions.push(completed);
    }
    completions
}

fn deferred_command_failure(command: Command, message: &str) -> Option<Event> {
    match command {
        Command::LeaveGroup {
            request_id,
            group_key,
        } => Some(Event::group_request_error(request_id, group_key, message)),
        Command::SendMessage { request_id, .. }
        | Command::SendGroupMessage { request_id, .. }
        | Command::AcceptIdentity { request_id, .. }
        | Command::DismissIdentity { request_id, .. }
        | Command::MarkRead { request_id, .. } => Some(Event::request_error(request_id, message)),
        Command::SendAttachment {
            request_id, permit, ..
        } if permit.claim_terminal() => Some(Event::request_error(request_id, message)),
        Command::SendAttachment { .. } => None,
        Command::SetTyping { .. } => None,
    }
}

fn fail_deferred_commands(sink: &EventSink, commands: &mut VecDeque<Command>, message: &str) {
    while let Some(command) = commands.pop_front() {
        if let Some(event) = deferred_command_failure(command, message) {
            sink.emit(event);
        }
    }
}

fn handle_recovery_command(command: Command, deferred_commands: &mut VecDeque<Command>) {
    match command {
        Command::SetTyping { .. } => {}
        command => {
            deferred_commands.push_back(command);
        }
    }
}

fn drain_recovery_commands(
    commands: &mut tokio_mpsc::Receiver<Command>,
    deferred_commands: &mut VecDeque<Command>,
) -> bool {
    loop {
        match commands.try_recv() {
            Ok(command) => handle_recovery_command(command, deferred_commands),
            Err(tokio_mpsc::error::TryRecvError::Empty) => return false,
            Err(tokio_mpsc::error::TryRecvError::Disconnected) => return true,
        }
    }
}

async fn handle_command_interruptibly(
    manager: &mut Manager<SqliteStore, Registered>,
    command: Command,
    shutdown: &mut watch::Receiver<bool>,
    sink: &EventSink,
    departed_groups: &DepartedGroups,
    groups_authoritative: bool,
    timestamps: &MessageTimestampAllocator,
) -> bool {
    let operation = handle_command(
        manager,
        command,
        sink,
        departed_groups,
        groups_authoritative,
        timestamps,
    );
    pin_mut!(operation);

    tokio::select! {
        () = &mut operation => false,
        _ = wait_for_shutdown(shutdown) => true,
    }
}

async fn emit_contact_snapshot(manager: &Manager<SqliteStore, Registered>, sink: &EventSink) {
    match manager.store().contacts().await {
        Ok(contacts) => match contacts.collect::<Result<Vec<_>, _>>() {
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
                    sink.emit(Event {
                        kind: EVENT_CONTACT,
                        peer_id: Some(peer),
                        title: (!contact.name.is_empty()).then_some(contact.name),
                        text: contact.phone_number.map(|number| number.to_string()),
                        ..Event::default()
                    });
                }
                sink.emit(Event {
                    kind: EVENT_CONTACT_SYNC_END,
                    ..Event::default()
                });
            }
            Err(error) => sink.emit(Event::error(
                format!("Could not decode synchronized Signal contacts: {error}"),
                false,
            )),
        },
        Err(error) => sink.emit(Event::error(
            format!("Could not read synchronized Signal contacts: {error}"),
            false,
        )),
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

async fn emit_account_identity(manager: &mut Manager<SqliteStore, Registered>, sink: &EventSink) {
    let local_aci = manager.registration_data().service_ids.aci();
    let profile_name = manager
        .retrieve_profile()
        .await
        .ok()
        .and_then(|profile| profile.name)
        .map(|name| name.to_string());

    sink.emit(account_identity_event(local_aci, profile_name));
}

async fn emit_group_snapshot(
    manager: &Manager<SqliteStore, Registered>,
    sink: &EventSink,
    departed_groups: &DepartedGroups,
) -> Result<(), String> {
    let groups = manager
        .store()
        .groups()
        .await
        .map_err(|error| format!("Could not read synchronized Signal groups: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not decode synchronized Signal groups: {error}"))?;

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
        sink.emit(Event {
            kind: EVENT_GROUP,
            chat_id: Some(chat_id.clone()),
            title: Some(group.title),
            ..Event::default()
        });
        emitted_records += 1;
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
) -> Result<(), String> {
    manager
        .synchronize_storage_groups()
        .await
        .map_err(|error| format!("Could not synchronize Signal groups: {error}"))?;
    emit_group_snapshot(manager, sink, departed_groups).await
}

async fn emit_identity_changes(manager: &Manager<SqliteStore, Registered>, sink: &EventSink) {
    match manager.store().identity_change_notices().await {
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
        Err(error) => sink.emit(Event::error(
            format!("Could not read Signal identity changes: {error}"),
            false,
        )),
    }
}

fn retry_delay_ms(attempts: u32) -> u64 {
    let exponent = attempts.min(9);
    5_000u64.saturating_mul(1u64 << exponent).min(3_600_000)
}

async fn attempt_outbox_message(
    manager: &mut Manager<SqliteStore, Registered>,
    message: &ClientOutboxMessage,
    departed_groups: &DepartedGroups,
) -> Result<SentMessage, OutboxAttemptError> {
    match message.kind {
        ClientOutboxKind::Direct => {
            let recipient = parse_recipient(&message.recipient).ok_or_else(|| {
                OutboxAttemptError::permanent(
                    "Recipient is not a canonical Signal service identifier",
                )
            })?;
            manager
                .send_message(
                    recipient,
                    DataMessage {
                        body: Some(message.body.clone()),
                        timestamp: Some(message.timestamp),
                        ..Default::default()
                    },
                    message.timestamp,
                )
                .await
                .map_err(|error| OutboxAttemptError::retryable(error.to_string()))?;
            Ok(SentMessage {
                thread: Thread::Contact(recipient),
                timestamp: message.timestamp,
            })
        }
        ClientOutboxKind::Group => {
            let (key, group) = resolve_active_group(manager, &message.recipient, departed_groups)
                .await
                .map_err(OutboxAttemptError::retryable)?
                .ok_or_else(|| {
                    OutboxAttemptError::permanent(
                        "Signal group is unavailable or this account is no longer a member",
                    )
                })?;
            manager
                .send_message_to_group(
                    &key,
                    DataMessage {
                        body: Some(message.body.clone()),
                        timestamp: Some(message.timestamp),
                        group_v2: Some(GroupContextV2 {
                            master_key: Some(key.to_vec()),
                            revision: Some(group.revision),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    message.timestamp,
                )
                .await
                .map_err(|error| OutboxAttemptError::retryable(error.to_string()))?;
            Ok(SentMessage {
                thread: Thread::Group(key),
                timestamp: message.timestamp,
            })
        }
    }
}

async fn mark_sent_message_projected(
    store: &SqliteStore,
    sent: &SentMessage,
) -> Result<(), String> {
    let content = store
        .message(&sent.thread, sent.timestamp)
        .await
        .map_err(|error| format!("Could not read the sent Signal message: {error}"))?
        .ok_or_else(|| "The sent Signal message was not found in the encrypted store".to_owned())?;
    store
        .mark_message_projected(MESSAGE_PROJECTION_CLIENT, &content)
        .await
        .map_err(|error| format!("Could not record the sent Signal message: {error}"))
}

async fn mark_sent_message_projected_or_report(
    manager: &Manager<SqliteStore, Registered>,
    sent: &SentMessage,
    sink: &EventSink,
) {
    if let Err(error) = mark_sent_message_projected(manager.store(), sent).await {
        sink.emit(Event::error(error, false));
    }
}

async fn finish_outbox_attempt(
    manager: &mut Manager<SqliteStore, Registered>,
    message: &ClientOutboxMessage,
    result: &Result<SentMessage, OutboxAttemptError>,
) -> Result<(), String> {
    match result {
        Ok(_) => manager
            .store()
            .complete_client_message(message.id)
            .await
            .map_err(|error| {
                format!("Message sent but its outbox entry could not be cleared: {error}")
            }),
        Err(error) if !error.should_retry() => manager
            .store()
            .complete_client_message(message.id)
            .await
            .map_err(|store_error| {
                format!("Could not discard a terminal outbox entry: {store_error}")
            }),
        Err(_) => {
            let attempts = message.attempts.saturating_add(1);
            manager
                .store()
                .defer_client_message(
                    message.id,
                    attempts,
                    wall_clock_ms().saturating_add(retry_delay_ms(attempts)),
                )
                .await
                .map_err(|error| format!("Could not schedule message retry: {error}"))
        }
    }
}

async fn retry_outbox(
    manager: &mut Manager<SqliteStore, Registered>,
    sink: &EventSink,
    departed_groups: &DepartedGroups,
    groups_authoritative: bool,
) {
    let messages = match manager.store().due_client_messages(wall_clock_ms()).await {
        Ok(messages) => messages,
        Err(error) => {
            sink.emit(Event::error(
                format!("Could not read the encrypted Signal outbox: {error}"),
                false,
            ));
            return;
        }
    };
    for message in messages {
        if !outbox_message_is_attemptable(&message.kind, groups_authoritative) {
            continue;
        }
        let result = attempt_outbox_message(manager, &message, departed_groups).await;
        if let Ok(sent) = &result {
            mark_sent_message_projected_or_report(manager, sent, sink).await;
        }
        if let Err(error) = finish_outbox_attempt(manager, &message, &result).await {
            sink.emit(Event::error(error, false));
        } else if let Err(error) = result {
            if !error.should_retry() {
                sink.emit(Event::error(
                    format!(
                        "Discarded a queued Signal message that can no longer be sent: {error}"
                    ),
                    false,
                ));
            } else if matches!(message.attempts.saturating_add(1), 4 | 8) {
                sink.emit(Event::error(
                    format!(
                        "A Signal message is still queued after {} attempts: {error}",
                        message.attempts.saturating_add(1)
                    ),
                    false,
                ));
            }
        }
    }
}

fn outbox_message_is_attemptable(kind: &ClientOutboxKind, groups_authoritative: bool) -> bool {
    groups_authoritative || matches!(kind, ClientOutboxKind::Direct)
}

async fn enqueue_and_send(
    manager: &mut Manager<SqliteStore, Registered>,
    kind: ClientOutboxKind,
    recipient: String,
    body: String,
    departed_groups: &DepartedGroups,
    sink: &EventSink,
    timestamps: &MessageTimestampAllocator,
) -> Result<(), String> {
    let timestamp = timestamps.next();
    let id = manager
        .store()
        .enqueue_client_message(kind, &recipient, &body, timestamp)
        .await
        .map_err(|error| format!("Could not save the message in the encrypted outbox: {error}"))?;
    let message = ClientOutboxMessage {
        id,
        kind,
        recipient,
        body,
        timestamp,
        attempts: 0,
    };
    let result = attempt_outbox_message(manager, &message, departed_groups).await;
    if let Ok(sent) = &result {
        mark_sent_message_projected_or_report(manager, sent, sink).await;
    }
    finish_outbox_attempt(manager, &message, &result).await?;
    result.map(|_| ()).map_err(|error| error.to_string())
}

struct OutgoingAttachment {
    recipient: String,
    filename: String,
    content_type: String,
    data: Vec<u8>,
    group: bool,
}

async fn upload_and_send_attachment(
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

async fn load_unprojected_messages(
    manager: &Manager<SqliteStore, Registered>,
    sink: &EventSink,
    replay: &mut MessageReplayQueue,
    groups_authoritative: bool,
) {
    let messages = match manager
        .store()
        .unprojected_messages(MESSAGE_PROJECTION_CLIENT)
        .await
    {
        Ok(messages) => messages,
        Err(error) => {
            sink.emit(Event::error(
                format!("Could not read pending Signal messages: {error}"),
                false,
            ));
            return;
        }
    };

    replay.replace(messages, groups_authoritative);
}

async fn project_content(
    manager: &mut Manager<SqliteStore, Registered>,
    content: Content,
    sink: &EventSink,
    projection: &mut MessageProjection,
    departed_groups: &DepartedGroups,
    groups_authoritative: bool,
    timestamps: &MessageTimestampAllocator,
) {
    if !content_is_projectable(&content.body, groups_authoritative) {
        return;
    }
    let Some(delivery_id) = projection.track(content.clone()) else {
        return;
    };
    let effect = projection_effect(
        handle_content(
            manager,
            content.clone(),
            delivery_id,
            sink,
            departed_groups,
            &mut projection.delivery_receipts,
            timestamps,
        )
        .await,
    );
    if !effect.remove_pending {
        return;
    }

    if !effect.mark_projected {
        projection.release(delivery_id);
        return;
    }
    match manager
        .store()
        .mark_message_projected(MESSAGE_PROJECTION_CLIENT, &content)
        .await
    {
        Ok(()) => {
            projection.complete(delivery_id);
        }
        Err(error) => {
            projection.release(delivery_id);
            sink.emit(Event::error(
                format!("Could not record a handled Signal message: {error}"),
                false,
            ));
        }
    }
}

fn content_has_group_context(content: &ContentBody) -> bool {
    match content {
        ContentBody::DataMessage(message) => message.group_v2.is_some(),
        ContentBody::EditMessage(EditMessage {
            data_message: Some(message),
            ..
        }) => message.group_v2.is_some(),
        ContentBody::SynchronizeMessage(SyncMessage {
            sent: Some(sent), ..
        }) => sent
            .message
            .as_ref()
            .or_else(|| {
                sent.edit_message
                    .as_ref()
                    .and_then(|edit| edit.data_message.as_ref())
            })
            .is_some_and(|message| message.group_v2.is_some()),
        _ => false,
    }
}

fn content_is_projectable(content: &ContentBody, groups_authoritative: bool) -> bool {
    groups_authoritative || !content_has_group_context(content)
}

async fn acknowledge_message(
    manager: &Manager<SqliteStore, Registered>,
    delivery_id: u64,
    sink: &EventSink,
    projection: &mut MessageProjection,
) -> bool {
    let Some(content) = projection.pending.get(&delivery_id) else {
        projection.acknowledgments.unregister(delivery_id);
        return true;
    };
    match manager
        .store()
        .mark_message_projected(MESSAGE_PROJECTION_CLIENT, content)
        .await
    {
        Ok(()) => {
            projection.complete(delivery_id);
            true
        }
        Err(error) => {
            sink.emit(Event::error(
                format!("Could not acknowledge a displayed Signal message: {error}"),
                false,
            ));
            false
        }
    }
}

async fn process_acknowledgments(
    manager: &Manager<SqliteStore, Registered>,
    acknowledgments: &AcknowledgmentInbox,
    sink: &EventSink,
    projection: &mut MessageProjection,
    retry_failures: bool,
) -> usize {
    const ACKNOWLEDGMENT_BATCH_SIZE: usize = 64;

    let delivery_ids = acknowledgments.take_ready(ACKNOWLEDGMENT_BATCH_SIZE);
    let count = delivery_ids.len();
    for delivery_id in delivery_ids {
        if !acknowledge_message(manager, delivery_id, sink, projection).await && retry_failures {
            acknowledgments.defer_retry(delivery_id);
        }
    }
    count
}

async fn drain_acknowledgments(
    manager: &Manager<SqliteStore, Registered>,
    acknowledgments: &AcknowledgmentInbox,
    sink: &EventSink,
    projection: &mut MessageProjection,
) {
    acknowledgments.close();
    while process_acknowledgments(manager, acknowledgments, sink, projection, false).await != 0 {}
}

async fn abort_delivery_receipt_tasks(
    tasks: &mut tokio::task::JoinSet<DeliveryReceiptCompletion>,
    receipts: &mut DeliveryReceiptQueue,
) {
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    receipts.release_in_flight();
}

async fn stop_attachments_and_drain_acknowledgments(
    manager: &Manager<SqliteStore, Registered>,
    sink: &EventSink,
    attachment_tasks: &mut tokio::task::JoinSet<AttachmentCompletion>,
    attachment_controls: &mut HashMap<u64, AttachmentTaskControl>,
    acknowledgments: &AcknowledgmentInbox,
    projection: &mut MessageProjection,
) {
    acknowledgments.close();
    let cleanup = async {
        abort_delivery_receipt_tasks(
            &mut projection.delivery_receipt_tasks,
            &mut projection.delivery_receipts,
        )
        .await;
        abort_in_flight_attachments(manager, sink, attachment_tasks, attachment_controls).await;
        drain_acknowledgments(manager, acknowledgments, sink, projection).await;
    };
    if !finish_shutdown_cleanup(cleanup, SHUTDOWN_CLEANUP_TIMEOUT).await {
        abandon_timed_out_attachments(sink, attachment_tasks, attachment_controls);
    }
}

async fn stop_receive_tasks<F>(
    tasks: &mut ActiveReceiveTasks,
    manager: &Manager<SqliteStore, Registered>,
    sink: &EventSink,
    attachment_tasks: &mut tokio::task::JoinSet<AttachmentCompletion>,
    attachment_controls: &mut HashMap<u64, AttachmentTaskControl>,
    final_cleanup: F,
) where
    F: Future<Output = ()>,
{
    tasks.receive.abort();
    tasks.contact_sync.abort();
    let cleanup = async {
        let _ = (&mut tasks.receive).await;
        let _ = (&mut tasks.contact_sync).await;
        abort_in_flight_attachments(manager, sink, attachment_tasks, attachment_controls).await;
        final_cleanup.await;
    };
    if !finish_shutdown_cleanup(cleanup, SHUTDOWN_CLEANUP_TIMEOUT).await {
        abandon_timed_out_attachments(sink, attachment_tasks, attachment_controls);
    }
}

async fn stop_active_receive_loop(
    tasks: &mut ActiveReceiveTasks,
    manager: &Manager<SqliteStore, Registered>,
    sink: &EventSink,
    attachment_tasks: &mut tokio::task::JoinSet<AttachmentCompletion>,
    attachment_controls: &mut HashMap<u64, AttachmentTaskControl>,
    acknowledgments: &AcknowledgmentInbox,
    projection: &mut MessageProjection,
) {
    acknowledgments.close();
    let final_cleanup = async {
        abort_delivery_receipt_tasks(
            &mut projection.delivery_receipt_tasks,
            &mut projection.delivery_receipts,
        )
        .await;
        drain_acknowledgments(manager, acknowledgments, sink, projection).await;
    };
    stop_receive_tasks(
        tasks,
        manager,
        sink,
        attachment_tasks,
        attachment_controls,
        final_cleanup,
    )
    .await;
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
        match manager.store().accept_identity_change(&recipient).await {
            Ok(true) => {
                if let Err(error) = manager.store().expedite_client_messages(&recipient).await {
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
        if let Err(error) = manager.store().dismiss_identity_change(&recipient).await {
            sink.emit(Event::request_error(
                request_id,
                format!("Could not dismiss the Signal identity notice: {error}"),
            ));
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
            sink.emit(Event::request_error(request_id, error));
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
                if let Err(error) = manager.store().expedite_client_messages(&group_key).await {
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
                enqueue_and_send(
                    manager,
                    ClientOutboxKind::Direct,
                    recipient,
                    message,
                    departed_groups,
                    sink,
                    timestamps,
                )
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
                        enqueue_and_send(
                            manager,
                            ClientOutboxKind::Group,
                            group_key,
                            message,
                            departed_groups,
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
                            },
                            timestamp,
                        )
                        .await
                        .map_err(|error| error.to_string())
                }
                None => Err("Recipient is not a canonical Signal service identifier".into()),
            };
            (request_id, result)
        }
        Command::SendAttachment { .. } => unreachable!(),
        Command::LeaveGroup { .. } => unreachable!(),
        Command::AcceptIdentity { .. } | Command::DismissIdentity { .. } => unreachable!(),
        Command::MarkRead { .. } => unreachable!(),
    };

    if let Err(error) = result {
        sink.emit(Event::request_error(request_id, error));
    }
}

async fn handle_content(
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

fn inline_image_matches(content_type: Option<&str>, data: &[u8]) -> bool {
    if data.len() > MAX_INLINE_MEDIA_BYTES {
        return false;
    }
    match content_type {
        Some(content_type) if content_type.eq_ignore_ascii_case("image/jpeg") => {
            data.starts_with(&[0xff, 0xd8, 0xff])
        }
        Some(content_type) if content_type.eq_ignore_ascii_case("image/png") => {
            data.starts_with(b"\x89PNG\r\n\x1a\n")
        }
        Some(content_type) if content_type.eq_ignore_ascii_case("image/gif") => {
            data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a")
        }
        _ => false,
    }
}

fn should_inline_image(outgoing: bool, content_type: Option<&str>, data: Option<&[u8]>) -> bool {
    !outgoing && data.is_some_and(|data| inline_image_matches(content_type, data))
}

fn gif_u16(data: &[u8], offset: usize) -> Option<usize> {
    let encoded: [u8; 2] = data.get(offset..offset.checked_add(2)?)?.try_into().ok()?;
    Some(u16::from_le_bytes(encoded) as usize)
}

fn advance_gif_offset(offset: &mut usize, amount: usize, size: usize) -> bool {
    if *offset > size || amount > size - *offset {
        return false;
    }
    *offset += amount;
    true
}

fn skip_gif_sub_blocks(data: &[u8], offset: &mut usize) -> bool {
    while *offset < data.len() {
        let block_size = data[*offset] as usize;
        *offset += 1;
        if block_size == 0 {
            return true;
        }
        if !advance_gif_offset(offset, block_size, data.len()) {
            return false;
        }
    }
    false
}

fn bounded_inline_gif(data: &[u8]) -> bool {
    if data.len() < 13
        || data.len() > MAX_INLINE_MEDIA_BYTES
        || !(data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a"))
    {
        return false;
    }

    let Some(canvas_width) = gif_u16(data, 6) else {
        return false;
    };
    let Some(canvas_height) = gif_u16(data, 8) else {
        return false;
    };
    let Some(canvas_pixels) = canvas_width.checked_mul(canvas_height) else {
        return false;
    };
    if canvas_width == 0
        || canvas_height == 0
        || canvas_width > MAX_INLINE_GIF_EDGE
        || canvas_height > MAX_INLINE_GIF_EDGE
        || canvas_pixels > MAX_INLINE_GIF_PIXELS
    {
        return false;
    }

    let mut offset = 13usize;
    let packed = data[10];
    if packed & 0x80 != 0 {
        let color_table_size = 3usize << ((packed & 0x07) as usize + 1);
        if !advance_gif_offset(&mut offset, color_table_size, data.len()) {
            return false;
        }
    }

    let mut frames = 0usize;
    while offset < data.len() {
        let marker = data[offset];
        offset += 1;
        match marker {
            0x3b => return frames > 0,
            0x21 => {
                if !advance_gif_offset(&mut offset, 1, data.len())
                    || !skip_gif_sub_blocks(data, &mut offset)
                {
                    return false;
                }
            }
            0x2c => {
                if offset > data.len() || 9 > data.len() - offset {
                    return false;
                }
                let Some(left) = gif_u16(data, offset) else {
                    return false;
                };
                let Some(top) = gif_u16(data, offset + 2) else {
                    return false;
                };
                let Some(width) = gif_u16(data, offset + 4) else {
                    return false;
                };
                let Some(height) = gif_u16(data, offset + 6) else {
                    return false;
                };
                let image_packed = data[offset + 8];
                offset += 9;

                if width == 0
                    || height == 0
                    || left
                        .checked_add(width)
                        .is_none_or(|right| right > canvas_width)
                    || top
                        .checked_add(height)
                        .is_none_or(|bottom| bottom > canvas_height)
                {
                    return false;
                }
                frames += 1;
                if frames > MAX_INLINE_GIF_PIXEL_FRAMES / canvas_pixels {
                    return false;
                }

                if image_packed & 0x80 != 0 {
                    let color_table_size = 3usize << ((image_packed & 0x07) as usize + 1);
                    if !advance_gif_offset(&mut offset, color_table_size, data.len()) {
                        return false;
                    }
                }
                if offset >= data.len() || !(2..=8).contains(&data[offset]) {
                    return false;
                }
                offset += 1;
                if !skip_gif_sub_blocks(data, &mut offset) {
                    return false;
                }
            }
            _ => return false,
        }
    }
    false
}

fn mp4_file_type_box_matches(data: &[u8]) -> bool {
    if data.len() < 16 || data.get(4..8) != Some(b"ftyp") {
        return false;
    }
    let Some(encoded_size) = data.get(..4) else {
        return false;
    };
    let Ok(encoded_size) = <[u8; 4]>::try_from(encoded_size) else {
        return false;
    };
    let box_size = u32::from_be_bytes(encoded_size) as usize;
    (16..=data.len()).contains(&box_size)
}

fn signal_gif_video_matches(attachment: &AttachmentPointer, data: &[u8]) -> bool {
    data.len() <= MAX_INLINE_MEDIA_BYTES
        && attachment
            .content_type
            .as_deref()
            .is_some_and(|content_type| content_type.eq_ignore_ascii_case("video/mp4"))
        && attachment.flags.unwrap_or_default() & attachment_pointer::Flags::Gif as u32 != 0
        && mp4_file_type_box_matches(data)
}

fn signal_gif_inline_filename(attachment: &AttachmentPointer) -> String {
    let Some(filename) = attachment
        .file_name
        .as_deref()
        .filter(|filename| !filename.is_empty())
    else {
        return "signal-animation.gif".to_owned();
    };
    let basename = filename.rsplit(['/', '\\']).next().unwrap_or_default();
    if basename.is_empty() || basename == "." || basename == ".." {
        return "signal-animation.gif".to_owned();
    }
    let stem = basename
        .rsplit_once('.')
        .and_then(|(stem, _)| (!stem.is_empty()).then_some(stem))
        .unwrap_or(basename);
    format!("{stem}.gif")
}

struct DownloadedAttachment {
    attachment_index: usize,
    filename: String,
    content_type: Option<String>,
    data: Vec<u8>,
    signal_gif_filename: Option<String>,
}

impl DownloadedAttachment {
    fn new(attachment_index: usize, attachment: &AttachmentPointer, data: Vec<u8>) -> Self {
        let signal_gif_filename = signal_gif_video_matches(attachment, &data)
            .then(|| signal_gif_inline_filename(attachment));
        Self {
            attachment_index,
            filename: attachment_display_name(attachment).to_owned(),
            content_type: attachment.content_type.clone(),
            data,
            signal_gif_filename,
        }
    }

    fn apply_signal_gif(&mut self, gif: Vec<u8>) -> bool {
        let Some(filename) = self.signal_gif_filename.clone() else {
            return false;
        };
        if !bounded_inline_gif(&gif) {
            return false;
        }

        self.filename = filename;
        self.signal_gif_filename = None;
        self.content_type = Some("image/gif".to_owned());
        self.data = gif;
        true
    }
}

fn stop_transcode_child(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn read_transcode_output(mut output: impl Read) -> Option<Vec<u8>> {
    let mut collected = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let read = output.read(&mut chunk).ok()?;
        if read == 0 {
            return Some(collected);
        }
        if read > MAX_INLINE_MEDIA_BYTES.saturating_sub(collected.len()) {
            return None;
        }
        collected.extend_from_slice(&chunk[..read]);
    }
}

fn signal_gif_transcode_stderr() -> std::process::Stdio {
    #[cfg(test)]
    if std::env::var_os("SIGNAL_PURPLE_REQUIRE_FFMPEG_TEST").is_some() {
        return std::process::Stdio::inherit();
    }
    std::process::Stdio::null()
}

fn transcode_signal_gif_video_blocking(input: Vec<u8>) -> Option<Vec<u8>> {
    let _permit = SIGNAL_GIF_TRANSCODE_LOCK.try_lock().ok()?;
    if !Path::new(SIGNAL_GIF_FFMPEG).is_file() || !Path::new(SIGNAL_GIF_PRLIMIT).is_file() {
        return None;
    }

    // Fixed Debian paths and arguments avoid shell or inherited PATH handling.
    // prlimit execs FFmpeg in the same child, so kill and wait cover both.
    let mut child = std::process::Command::new(SIGNAL_GIF_PRLIMIT)
        .args([
            SIGNAL_GIF_ADDRESS_SPACE_LIMIT,
            "--cpu=10:12",
            "--nofile=64:64",
            "--",
            SIGNAL_GIF_FFMPEG,
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-max_alloc",
            "134217728",
            "-threads",
            "1",
            "-filter_threads",
            "1",
            "-filter_complex_threads",
            "1",
            "-protocol_whitelist",
            "pipe",
            "-probesize",
            "8388608",
            "-analyzeduration",
            "5000000",
            "-i",
            "pipe:0",
            "-map",
            "0:v:0",
            "-map_metadata",
            "-1",
            "-map_chapters",
            "-1",
            "-an",
            "-sn",
            "-dn",
            "-vf",
            "scale=w='min(480,iw)':h='min(480,ih)':force_original_aspect_ratio=decrease:flags=lanczos",
            "-fpsmax",
            "15",
            "-threads",
            "1",
            "-loop",
            "0",
            "-f",
            "gif",
            "pipe:1",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(signal_gif_transcode_stderr())
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LANG", "C")
        .spawn()
        .ok()?;

    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        stop_transcode_child(&mut child);
        return None;
    };
    let writer = match std::thread::Builder::new()
        .name("signal-gif-input".to_owned())
        .spawn(move || stdin.write_all(&input).is_ok())
    {
        Ok(writer) => writer,
        Err(_) => {
            stop_transcode_child(&mut child);
            return None;
        }
    };
    let reader = match std::thread::Builder::new()
        .name("signal-gif-output".to_owned())
        .spawn(move || read_transcode_output(stdout))
    {
        Ok(reader) => reader,
        Err(_) => {
            stop_transcode_child(&mut child);
            let _ = writer.join();
            return None;
        }
    };

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < SIGNAL_GIF_TRANSCODE_TIMEOUT => {
                std::thread::sleep(SIGNAL_GIF_TRANSCODE_POLL_INTERVAL);
            }
            Ok(None) | Err(_) => {
                stop_transcode_child(&mut child);
                break None;
            }
        }
    };
    let input_complete = writer.join().unwrap_or(false);
    let output = reader.join().ok().flatten();
    if !input_complete || !status.is_some_and(|status| status.success()) {
        return None;
    }
    output.filter(|output| bounded_inline_gif(output))
}

async fn transcode_signal_gif_video(input: &[u8]) -> Option<Vec<u8>> {
    // Keep the caller's original bytes untouched for the receive-file fallback.
    let input = input.to_vec();
    tokio::task::spawn_blocking(move || transcode_signal_gif_video_blocking(input))
        .await
        .ok()
        .flatten()
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

fn attachment_display_name(attachment: &AttachmentPointer) -> &str {
    if let Some(file_name) = attachment
        .file_name
        .as_deref()
        .filter(|file_name| !file_name.is_empty())
    {
        return file_name;
    }

    let content_type = attachment.content_type.as_deref();
    let is_gif = attachment.flags.unwrap_or_default() & attachment_pointer::Flags::Gif as u32 != 0;
    if is_gif {
        if content_type.is_some_and(|value| value.eq_ignore_ascii_case("image/gif")) {
            return "signal-animation.gif";
        }
        if content_type.is_some_and(|value| value.eq_ignore_ascii_case("video/mp4")) {
            return "signal-animation.mp4";
        }
        return "signal-animation";
    }

    match content_type {
        Some(value) if value.eq_ignore_ascii_case("image/jpeg") => "signal-image.jpg",
        Some(value) if value.eq_ignore_ascii_case("image/png") => "signal-image.png",
        Some(value) if value.eq_ignore_ascii_case("image/gif") => "signal-animation.gif",
        Some(value) if value.eq_ignore_ascii_case("video/mp4") => "signal-video.mp4",
        _ => "signal-attachment",
    }
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
            match manager.get_attachment(attachment).await {
                Ok(data) if data.is_empty() => sink.emit(Event::error(
                    "Could not download a Signal attachment: decrypted attachment was empty",
                    false,
                )),
                Ok(data) => downloaded.push(DownloadedAttachment::new(
                    attachment_index,
                    attachment,
                    data,
                )),
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

fn spawn_delivery_receipt_attempt(
    tasks: &mut tokio::task::JoinSet<DeliveryReceiptCompletion>,
    manager: Manager<SqliteStore, Registered>,
    generation: u64,
    receipt: PendingDeliveryReceipt,
) {
    tasks.spawn_local(async move {
        let mut manager = manager;
        let result = send_receipt_at_timestamp(
            &mut manager,
            receipt.recipient,
            receipt.identity.message_timestamp,
            receipt_message::Type::Delivery,
            receipt.send_timestamp,
        )
        .await;
        DeliveryReceiptCompletion {
            generation,
            receipt,
            result,
        }
    });
}

fn finish_delivery_receipt_attempt(
    receipts: &mut DeliveryReceiptQueue,
    sink: &EventSink,
    completion: &DeliveryReceiptCompletion,
) -> Option<String> {
    let Err(error) = &completion.result else {
        receipts.complete(&completion.receipt.identity);
        return None;
    };
    match delivery_receipt_failure_action(error) {
        DeliveryReceiptFailureAction::Retry => {
            receipts.retry(&completion.receipt.identity, false);
            None
        }
        DeliveryReceiptFailureAction::Recover => {
            receipts.retry(&completion.receipt.identity, true);
            Some(format!(
                "Signal websocket closed while sending a delivery receipt: {error}"
            ))
        }
        DeliveryReceiptFailureAction::Discard => {
            receipts.complete(&completion.receipt.identity);
            sink.emit(Event::error(
                format!("Could not send a Signal delivery receipt: {error}"),
                false,
            ));
            None
        }
    }
}

async fn send_receipt_at_timestamp(
    manager: &mut Manager<SqliteStore, Registered>,
    recipient: ServiceId,
    message_timestamp: u64,
    receipt_type: receipt_message::Type,
    send_timestamp: u64,
) -> Result<(), presage::Error<presage_store_sqlite::SqliteStoreError>> {
    manager
        .send_message(
            recipient,
            ReceiptMessage {
                r#type: Some(receipt_type.into()),
                timestamp: vec![message_timestamp],
            },
            send_timestamp,
        )
        .await
}

async fn send_receipt(
    manager: &mut Manager<SqliteStore, Registered>,
    recipient: ServiceId,
    message_timestamp: u64,
    receipt_type: receipt_message::Type,
    timestamps: &MessageTimestampAllocator,
) -> Result<(), presage::Error<presage_store_sqlite::SqliteStoreError>> {
    send_receipt_at_timestamp(
        manager,
        recipient,
        message_timestamp,
        receipt_type,
        timestamps.next(),
    )
    .await
}

fn parse_recipient(value: &str) -> Option<ServiceId> {
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

    let local_aci = manager.registration_data().service_ids.aci();
    let group = manager
        .store()
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
    manager
        .store()
        .group(key)
        .await
        .map(|group| group.filter(|group| group_contains_local_aci(group, &local_aci)))
        .map_err(|error| format!("Could not read Signal group membership: {error}"))
}

async fn resolve_active_group(
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
    let groups = manager
        .store()
        .groups()
        .await
        .map_err(|error| format!("Could not read Signal groups: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not decode Signal groups: {error}"))?;
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

fn wall_clock_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn qr_png(value: &[u8]) -> Result<Vec<u8>, String> {
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
    use std::path::PathBuf;

    fn projection_test_content(timestamp_ms: u64, group: bool) -> Content {
        let sender = parse_recipient("11111111-1111-4111-8111-111111111111").unwrap();
        let destination = parse_recipient("22222222-2222-4222-8222-222222222222").unwrap();
        let timestamp = (UNIX_EPOCH + Duration::from_millis(timestamp_ms)).into();
        Content {
            metadata: presage::libsignal_service::content::Metadata {
                sender,
                destination,
                sender_device: 1u32.try_into().unwrap(),
                timestamp,
                server_timestamp: timestamp,
                needs_receipt: false,
                unidentified_sender: false,
                was_plaintext: false,
                server_guid: None,
            },
            body: ContentBody::DataMessage(DataMessage {
                timestamp: Some(timestamp_ms),
                group_v2: group.then(GroupContextV2::default),
                ..DataMessage::default()
            }),
        }
    }

    struct DropFlag(Arc<AtomicBool>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            static NEXT_ID: AtomicU64 = AtomicU64::new(0);

            loop {
                let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir()
                    .join(format!("signal-purple-{label}-{}-{id}", std::process::id()));
                match std::fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => panic!("could not create test directory: {error}"),
                }
            }
        }

        fn join(&self, path: &str) -> PathBuf {
            self.0.join(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn shutdown_boundary_returns_completed_phase_output() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (_shutdown_tx, mut shutdown_rx) = watch::channel(false);

            assert_eq!(
                await_or_shutdown(async { 42 }, &mut shutdown_rx).await,
                Some(42)
            );
        });
    }

    #[test]
    fn shutdown_boundary_does_not_poll_after_shutdown() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
            let polled = Arc::new(AtomicBool::new(false));
            let phase_polled = Arc::clone(&polled);
            shutdown_tx.send(true).unwrap();

            let outcome = await_or_shutdown(
                async move {
                    phase_polled.store(true, Ordering::Release);
                    42
                },
                &mut shutdown_rx,
            )
            .await;

            assert_eq!(outcome, None);
            assert!(!polled.load(Ordering::Acquire));
        });
    }

    #[test]
    fn contact_sync_work_waits_for_the_queue_drain_signal() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (start_tx, start_rx) = oneshot::channel();
            let polled = Arc::new(AtomicBool::new(false));
            let operation_polled = Arc::clone(&polled);
            let gated = run_after_start_signal(start_rx, async move {
                operation_polled.store(true, Ordering::Release);
            });
            pin_mut!(gated);

            assert!(
                tokio::time::timeout(Duration::from_millis(10), gated.as_mut())
                    .await
                    .is_err()
            );
            assert!(!polled.load(Ordering::Acquire));

            start_tx.send(()).unwrap();
            tokio::time::timeout(Duration::from_secs(1), gated.as_mut())
                .await
                .expect("contact sync gate did not open");
            assert!(polled.load(Ordering::Acquire));
        });
    }

    #[test]
    fn receive_forwarder_keeps_store_owner_scheduled_while_actor_waits() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        runtime.block_on(local.run_until(async {
            let store_slot = Arc::new(tokio::sync::Semaphore::new(1));
            let stream_store_slot = Arc::clone(&store_slot);
            let (stream_started_tx, stream_started_rx) = oneshot::channel();
            let (release_stream_tx, release_stream_rx) = oneshot::channel();
            let stream = futures::stream::once(async move {
                let permit = stream_store_slot.acquire_owned().await.unwrap();
                stream_started_tx.send(()).unwrap();
                release_stream_rx.await.unwrap();
                drop(permit);
                42
            });
            let (messages_tx, mut messages_rx) = tokio_mpsc::channel(1);
            let forwarder =
                tokio::task::spawn_local(forward_stream_to_channel(stream, messages_tx));

            stream_started_rx.await.unwrap();
            let release = tokio::task::spawn_local(async move {
                tokio::time::sleep(Duration::from_millis(10)).await;
                release_stream_tx.send(()).unwrap();
            });
            let actor_permit = tokio::time::timeout(Duration::from_secs(1), store_slot.acquire())
                .await
                .expect("the independently polled receive stream retained the store slot")
                .unwrap();
            drop(actor_permit);

            assert_eq!(messages_rx.recv().await, Some(42));
            release.await.unwrap();
            forwarder.await.unwrap();
        }));
    }

    #[test]
    fn shutdown_boundary_drops_a_pending_phase_promptly() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
            let (started_tx, started_rx) = oneshot::channel();
            let dropped = Arc::new(AtomicBool::new(false));
            let phase_dropped = Arc::clone(&dropped);
            let phase = async move {
                let _drop_flag = DropFlag(phase_dropped);
                let _ = started_tx.send(());
                std::future::pending::<()>().await;
            };
            let signal_shutdown = async move {
                started_rx.await.expect("phase did not start");
                shutdown_tx.send(true).expect("shutdown receiver closed");
            };

            let outcome = tokio::time::timeout(Duration::from_secs(1), async {
                tokio::join!(await_or_shutdown(phase, &mut shutdown_rx), signal_shutdown).0
            })
            .await
            .expect("shutdown boundary did not complete");

            assert_eq!(outcome, None);
            assert!(dropped.load(Ordering::Acquire));
        });
    }

    #[test]
    fn shutdown_cleanup_drops_work_after_its_deadline() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let dropped = Arc::new(AtomicBool::new(false));
            let cleanup_dropped = Arc::clone(&dropped);
            let cleanup = async move {
                let _drop_flag = DropFlag(cleanup_dropped);
                std::future::pending::<()>().await;
            };

            let completed = finish_shutdown_cleanup(cleanup, Duration::from_millis(10)).await;

            assert!(!completed);
            assert!(dropped.load(Ordering::Acquire));
        });
    }

    #[test]
    fn runtime_shutdown_stops_waiting_for_blocking_work_after_its_deadline() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (finished_tx, finished_rx) = std::sync::mpsc::channel();
        runtime.block_on(async {
            let task = tokio::task::spawn_blocking(move || {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                finished_tx.send(()).unwrap();
            });
            started_rx.recv().unwrap();
            drop(task);
        });

        let started = std::time::Instant::now();
        shutdown_runtime(runtime, Duration::from_millis(10));
        assert!(started.elapsed() < Duration::from_secs(1));

        release_tx.send(()).unwrap();
        finished_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    }

    #[test]
    fn runtime_shutdown_is_bounded_after_worker_panic() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (finished_tx, finished_rx) = std::sync::mpsc::channel();
        let future = async move {
            let task = tokio::task::spawn_blocking(move || {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                finished_tx.send(()).unwrap();
            });
            started_rx.recv().unwrap();
            drop(task);
            panic!("test worker panic");
        };

        let started = std::time::Instant::now();
        let result = run_local_future(runtime, local, future, Duration::from_millis(10));
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(1));

        release_tx.send(()).unwrap();
        finished_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    }

    #[test]
    fn encrypted_store_open_drops_passphrase_before_returning() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let directory = TestDirectory::new("credential-lifetime");
        let store_path = directory.join("store.db3");
        runtime.block_on(async {
            let (_shutdown_tx, mut shutdown_rx) = watch::channel(false);
            let dropped = Arc::new(AtomicBool::new(false));
            let mut passphrase = StorePassphrase::new("test-store-passphrase".to_owned());
            passphrase.observe_drop(Arc::clone(&dropped));

            let store =
                open_encrypted_store(store_path.to_str().unwrap(), passphrase, &mut shutdown_rx)
                    .await
                    .unwrap()
                    .expect("store opening was interrupted");

            assert!(dropped.load(Ordering::Acquire));
            drop(store);
        });
    }

    #[test]
    fn encrypted_store_shutdown_drops_passphrase_without_polling_open() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
            let dropped = Arc::new(AtomicBool::new(false));
            let mut passphrase = StorePassphrase::new("test-store-passphrase".to_owned());
            passphrase.observe_drop(Arc::clone(&dropped));
            shutdown_tx.send(true).unwrap();

            let store = open_encrypted_store("/not/polled", passphrase, &mut shutdown_rx)
                .await
                .unwrap();

            assert!(store.is_none());
            assert!(dropped.load(Ordering::Acquire));
        });
    }

    #[test]
    fn encrypted_store_error_drops_passphrase_before_returning() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let directory = TestDirectory::new("credential-error");
        let store_path = directory.join("missing-parent").join("store.db3");
        runtime.block_on(async {
            let (_shutdown_tx, mut shutdown_rx) = watch::channel(false);
            let dropped = Arc::new(AtomicBool::new(false));
            let mut passphrase = StorePassphrase::new("test-store-passphrase".to_owned());
            passphrase.observe_drop(Arc::clone(&dropped));

            let result =
                open_encrypted_store(store_path.to_str().unwrap(), passphrase, &mut shutdown_rx)
                    .await;

            assert!(result.is_err());
            assert!(dropped.load(Ordering::Acquire));
        });
    }

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
    fn classifies_inactive_group_outbox_entries_as_terminal() {
        let terminal = OutboxAttemptError::permanent("not a member");
        let transient = OutboxAttemptError::retryable("network unavailable");

        assert!(!terminal.should_retry());
        assert!(transient.should_retry());
    }

    #[test]
    fn quarantines_group_outbox_until_membership_is_authoritative() {
        assert!(outbox_message_is_attemptable(
            &ClientOutboxKind::Direct,
            false
        ));
        assert!(!outbox_message_is_attemptable(
            &ClientOutboxKind::Group,
            false
        ));
        assert!(outbox_message_is_attemptable(
            &ClientOutboxKind::Group,
            true
        ));
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

    fn encoded_gif(width: u16, height: u16, frames: usize) -> Vec<u8> {
        let mut data = b"GIF89a".to_vec();
        data.extend_from_slice(&width.to_le_bytes());
        data.extend_from_slice(&height.to_le_bytes());
        data.extend_from_slice(&[0x80, 0x00, 0x00]);
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0xff, 0xff, 0xff]);
        for _ in 0..frames {
            data.push(0x2c);
            data.extend_from_slice(&0u16.to_le_bytes());
            data.extend_from_slice(&0u16.to_le_bytes());
            data.extend_from_slice(&width.to_le_bytes());
            data.extend_from_slice(&height.to_le_bytes());
            data.push(0x00);
            data.extend_from_slice(&[0x02, 0x02, 0x44, 0x01, 0x00]);
        }
        data.push(0x3b);
        data
    }

    fn signal_gif_mp4() -> Vec<u8> {
        let mut data = 24u32.to_be_bytes().to_vec();
        data.extend_from_slice(b"ftypisom");
        data.extend_from_slice(&0u32.to_be_bytes());
        data.extend_from_slice(b"isomiso2");
        data
    }

    #[test]
    fn recognizes_only_bounded_signal_gif_mp4_payloads() {
        let animation = AttachmentPointer {
            content_type: Some("video/mp4".into()),
            flags: Some(attachment_pointer::Flags::Gif as u32),
            ..AttachmentPointer::default()
        };
        let uppercase = AttachmentPointer {
            content_type: Some("VIDEO/MP4".into()),
            flags: Some(attachment_pointer::Flags::Gif as u32),
            ..AttachmentPointer::default()
        };
        let video = AttachmentPointer {
            content_type: Some("video/mp4".into()),
            ..AttachmentPointer::default()
        };
        let mp4 = signal_gif_mp4();

        assert!(signal_gif_video_matches(&animation, &mp4));
        assert!(signal_gif_video_matches(&uppercase, &mp4));
        assert!(!signal_gif_video_matches(&video, &mp4));
        assert!(!signal_gif_video_matches(&animation, b"not an mp4"));

        let mut invalid_box = mp4.clone();
        invalid_box[..4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(!signal_gif_video_matches(&animation, &invalid_box));

        let mut oversized = vec![0u8; MAX_INLINE_MEDIA_BYTES + 1];
        oversized[..mp4.len()].copy_from_slice(&mp4);
        assert!(!signal_gif_video_matches(&animation, &oversized));
    }

    #[test]
    fn validates_generated_gif_structure_and_frame_budget() {
        let gif = encoded_gif(1, 1, 2);
        let excessive_frames = encoded_gif(1000, 1000, 9);
        let mut truncated = gif.clone();
        truncated.pop();
        let mut oversized = gif.clone();
        oversized.resize(MAX_INLINE_MEDIA_BYTES + 1, 0);

        assert!(bounded_inline_gif(&gif));
        assert!(!bounded_inline_gif(&excessive_frames));
        assert!(!bounded_inline_gif(&truncated));
        assert!(!bounded_inline_gif(&oversized));
        assert!(!bounded_inline_gif(b"GIF89a"));
    }

    #[test]
    fn bounds_streamed_transcode_output_before_appending() {
        let output = b"bounded output";
        assert_eq!(
            read_transcode_output(std::io::Cursor::new(output)),
            Some(output.to_vec())
        );
        assert!(
            read_transcode_output(std::io::Read::take(
                std::io::repeat(0),
                (MAX_INLINE_MEDIA_BYTES + 1) as u64
            ))
            .is_none()
        );
    }

    #[test]
    fn installed_ffmpeg_converter_produces_a_bounded_animation() {
        if !Path::new(SIGNAL_GIF_FFMPEG).is_file() || !Path::new(SIGNAL_GIF_PRLIMIT).is_file() {
            assert!(
                std::env::var_os("SIGNAL_PURPLE_REQUIRE_FFMPEG_TEST").is_none(),
                "CI requires /usr/bin/ffmpeg and /usr/bin/prlimit for converter coverage"
            );
            return;
        }
        let source = std::process::Command::new(SIGNAL_GIF_FFMPEG)
            .args([
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=16x16:rate=2:duration=1",
                "-frames:v",
                "2",
                "-an",
                "-c:v",
                "mpeg4",
                "-pix_fmt",
                "yuv420p",
                "-movflags",
                "frag_keyframe+empty_moov",
                "-f",
                "mp4",
                "pipe:1",
            ])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("LANG", "C")
            .output()
            .expect("installed FFmpeg did not start");
        assert!(source.status.success());
        assert!(mp4_file_type_box_matches(&source.stdout));

        let gif = transcode_signal_gif_video_blocking(source.stdout)
            .expect("installed FFmpeg did not produce a bounded GIF");
        assert!(bounded_inline_gif(&gif));
        assert!(gif.starts_with(b"GIF89a"));
    }

    #[test]
    fn applies_only_valid_signal_gif_presentations() {
        let attachment = AttachmentPointer {
            file_name: Some("../../shared.MP4".into()),
            content_type: Some("video/mp4".into()),
            flags: Some(attachment_pointer::Flags::Gif as u32),
            ..AttachmentPointer::default()
        };
        let original = signal_gif_mp4();
        let gif = encoded_gif(1, 1, 2);
        let mut downloaded = DownloadedAttachment::new(3, &attachment, original.clone());

        assert_eq!(
            downloaded.signal_gif_filename.as_deref(),
            Some("shared.gif")
        );
        assert!(downloaded.apply_signal_gif(gif.clone()));
        assert_eq!(downloaded.filename, "shared.gif");
        assert_eq!(downloaded.content_type.as_deref(), Some("image/gif"));
        assert_eq!(downloaded.data, gif);

        let mut invalid = DownloadedAttachment::new(4, &attachment, original.clone());
        assert!(!invalid.apply_signal_gif(b"GIF89a".to_vec()));
        assert_eq!(invalid.data, original);
        assert_eq!(invalid.content_type.as_deref(), Some("video/mp4"));

        let direct = DownloadedAttachment::new(6, &attachment, signal_gif_mp4());
        assert_eq!(direct.signal_gif_filename.as_deref(), Some("shared.gif"));
    }

    #[test]
    fn recognizes_only_declared_image_payloads_for_inline_display() {
        let jpeg = [0xff, 0xd8, 0xff, 0xe0];
        let png = b"\x89PNG\r\n\x1a\nrest";
        let gif87a = b"GIF87arest";
        let gif89a = b"GIF89arest";

        assert!(inline_image_matches(Some("image/jpeg"), &jpeg));
        assert!(inline_image_matches(Some("IMAGE/JPEG"), &jpeg));
        assert!(inline_image_matches(Some("image/png"), png));
        assert!(inline_image_matches(Some("IMAGE/PNG"), png));
        assert!(inline_image_matches(Some("image/gif"), gif87a));
        assert!(inline_image_matches(Some("IMAGE/GIF"), gif89a));

        assert!(!inline_image_matches(Some("image/png"), &jpeg));
        assert!(!inline_image_matches(Some("image/jpeg"), png));
        assert!(!inline_image_matches(Some("image/png"), b"\x89PNG"));
        assert!(!inline_image_matches(Some("image/gif"), b"GIF89"));
        assert!(!inline_image_matches(
            Some("image/jpeg; charset=binary"),
            &jpeg
        ));
        assert!(!inline_image_matches(None, &jpeg));

        let mut oversized = vec![0u8; MAX_INLINE_MEDIA_BYTES + 1];
        oversized[..jpeg.len()].copy_from_slice(&jpeg);
        assert!(!inline_image_matches(Some("image/jpeg"), &oversized));
    }

    #[test]
    fn inlines_downloaded_images_in_every_incoming_conversation() {
        let jpeg = [0xff, 0xd8, 0xff, 0xe0];

        assert!(should_inline_image(false, Some("image/jpeg"), Some(&jpeg)));
        assert!(!should_inline_image(true, Some("image/jpeg"), Some(&jpeg)));
        assert!(!should_inline_image(
            false,
            Some("application/octet-stream"),
            Some(&jpeg)
        ));
        assert!(!should_inline_image(false, Some("image/jpeg"), None));
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
    fn supplies_useful_names_for_unnamed_media_attachments() {
        let video = AttachmentPointer {
            content_type: Some("video/mp4".into()),
            ..AttachmentPointer::default()
        };
        let animation = AttachmentPointer {
            content_type: Some("video/mp4".into()),
            flags: Some(attachment_pointer::Flags::Gif as u32),
            ..AttachmentPointer::default()
        };
        let named = AttachmentPointer {
            file_name: Some("shared clip".into()),
            content_type: Some("video/mp4".into()),
            flags: Some(attachment_pointer::Flags::Gif as u32),
            ..AttachmentPointer::default()
        };

        assert_eq!(attachment_display_name(&video), "signal-video.mp4");
        assert_eq!(attachment_display_name(&animation), "signal-animation.mp4");
        assert_eq!(attachment_display_name(&named), "shared clip");
        assert_eq!(
            attachment_display_name(&AttachmentPointer::default()),
            "signal-attachment"
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
    fn bounds_outbox_retry_backoff() {
        assert_eq!(retry_delay_ms(0), 5_000);
        assert_eq!(retry_delay_ms(1), 10_000);
        assert_eq!(retry_delay_ms(4), 80_000);
        assert_eq!(retry_delay_ms(32), 2_560_000);
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
    fn preserves_completed_attachment_results_when_aborting_a_generation() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut tasks = tokio::task::JoinSet::new();
            let mut aborts = HashMap::new();
            let (completed_tx, completed_rx) = oneshot::channel();

            let completed = tasks.spawn(async move {
                let _ = completed_tx.send(());
                (41, "sent")
            });
            aborts.insert(41, completed);
            let pending = tasks.spawn(async {
                futures::future::pending::<()>().await;
                (42, "sent")
            });
            aborts.insert(42, pending);

            completed_rx.await.unwrap();
            let results = abort_and_drain_tasks(&mut tasks, aborts.values()).await;
            let mut completed_ids = Vec::new();
            let mut cancelled = 0;
            for result in results {
                match result {
                    Ok((request_id, _)) => completed_ids.push(request_id),
                    Err(error) if error.is_cancelled() => cancelled += 1,
                    Err(error) => panic!("unexpected task failure: {error}"),
                }
            }

            assert_eq!(completed_ids, [41]);
            assert_eq!(cancelled, 1);
        });
    }

    #[test]
    fn attachment_task_panics_keep_the_request_identity() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let admission = AttachmentAdmission::for_test(1, 1);
        let completed = runtime.block_on(async {
            let mut tasks = tokio::task::JoinSet::new();
            tasks.spawn(attachment_task_result(
                41,
                admission.try_reserve(41, 1).unwrap(),
                async {
                    panic!("test attachment panic");
                },
            ));
            tasks.join_next().await.unwrap()
        });
        let Ok(AttachmentCompletion {
            request_id, result, ..
        }) = completed
        else {
            panic!("attachment panic escaped its task boundary");
        };

        assert_eq!(request_id, 41);
        let AttachmentTaskResult::Finished(Err(error)) = result else {
            panic!("panicking attachment task unexpectedly succeeded");
        };
        assert_eq!(error, "Signal attachment task failed unexpectedly");
    }

    #[test]
    fn cancellation_overtakes_a_queued_attachment() {
        let admission = AttachmentAdmission::for_test(1024, 1);
        let mut commands = VecDeque::from([
            Command::SendMessage {
                request_id: 40,
                recipient: "recipient".into(),
                message: "message".into(),
            },
            Command::SendAttachment {
                request_id: 41,
                recipient: "recipient".into(),
                filename: "attachment.txt".into(),
                content_type: "text/plain".into(),
                data: b"attachment".to_vec(),
                group: false,
                permit: admission.try_reserve(41, b"attachment".len()).unwrap(),
            },
        ]);

        assert!(admission.cancel(41));
        assert!(matches!(
            commands.pop_front(),
            Some(Command::SendMessage { request_id: 40, .. })
        ));
        let Some(Command::SendAttachment { permit, .. }) = commands.pop_front() else {
            panic!("queued attachment was lost");
        };
        assert!(permit.is_cancelled());
        drop(permit);
        assert_eq!(admission.usage(), (0, 0));
    }

    #[test]
    fn cancellation_stops_an_active_attachment_task_and_releases_admission() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let admission = AttachmentAdmission::for_test(1024, 1);
        let permit = admission.try_reserve(41, 10).unwrap();
        let cancellation_admission = Arc::clone(&admission);
        let (started_tx, started_rx) = oneshot::channel();
        let polls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let task_polls = Arc::clone(&polls);

        let (completion, ()) = runtime.block_on(async {
            tokio::join!(
                attachment_task_result(41, permit, {
                    let mut started_tx = Some(started_tx);
                    futures::future::poll_fn(move |_context| {
                        task_polls.fetch_add(1, Ordering::Relaxed);
                        if let Some(started_tx) = started_tx.take() {
                            let _ = started_tx.send(());
                        }
                        std::task::Poll::<Result<SentMessage, String>>::Pending
                    })
                }),
                async move {
                    started_rx.await.unwrap();
                    assert!(cancellation_admission.cancel(41));
                },
            )
        });

        assert!(matches!(completion.result, AttachmentTaskResult::Cancelled));
        assert_eq!(polls.load(Ordering::Relaxed), 1);
        assert_eq!(admission.usage(), (10, 1));
        drop(completion);
        assert_eq!(admission.usage(), (0, 0));
    }

    #[test]
    fn recovery_reports_every_non_cancelled_unreported_attachment() {
        let active_admission = AttachmentAdmission::for_test(2, 1);
        let active = active_admission.try_reserve(41, 1).unwrap();
        assert!(interrupted_attachment_event(41, &active.control()).is_some());

        let cancelled_admission = AttachmentAdmission::for_test(2, 1);
        let cancelled = cancelled_admission.try_reserve(42, 1).unwrap();
        assert!(cancelled_admission.cancel(42));
        assert!(interrupted_attachment_event(42, &cancelled.control()).is_none());

        let terminal_admission = AttachmentAdmission::for_test(2, 1);
        let terminal = terminal_admission.try_reserve(43, 1).unwrap();
        assert!(terminal.claim_terminal());
        assert!(interrupted_attachment_event(43, &terminal.control()).is_some());
    }

    #[test]
    fn completed_attachment_reports_terminal_event_before_projection() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(async {
            let admission = AttachmentAdmission::for_test(2, 1);
            let permit = admission.try_reserve(41, 1).unwrap();
            let control = permit.control();
            assert!(permit.claim_terminal());
            let mut tasks = tokio::task::JoinSet::new();
            let task = tasks.spawn(std::future::pending::<()>());
            let mut controls = HashMap::from([(41, AttachmentTaskControl { task, control })]);
            let (sink, queue) = crate::event_queue::event_queue(1).unwrap();
            let sent = finish_attachment_completion(
                &sink,
                &mut controls,
                Ok(AttachmentCompletion {
                    request_id: 41,
                    result: AttachmentTaskResult::Finished(Ok(SentMessage {
                        thread: Thread::Group([0; 32]),
                        timestamp: 1,
                    })),
                    permit,
                }),
            );

            assert!(sent.is_some());
            assert!(controls.is_empty());
            let crate::event_queue::EventPoll::Event(event) = queue.poll() else {
                panic!("expected an attachment terminal event");
            };
            assert_eq!(event.kind, EVENT_ATTACHMENT_SENT);
            assert_eq!(event.request_id, 41);

            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
        });
    }

    #[test]
    fn timed_out_cleanup_discards_ready_terminal_completions() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(async {
            let admission = AttachmentAdmission::for_test(2, 1);
            let permit = admission.try_reserve(41, 1).unwrap();
            let control = permit.control();
            assert!(permit.claim_terminal());
            let (ready_tx, ready_rx) = oneshot::channel();
            let mut tasks = tokio::task::JoinSet::new();
            let task = tasks.spawn(async move {
                let _ = ready_tx.send(());
                AttachmentCompletion {
                    request_id: 41,
                    result: AttachmentTaskResult::Finished(Ok(SentMessage {
                        thread: Thread::Group([0; 32]),
                        timestamp: 1,
                    })),
                    permit,
                }
            });
            let mut controls = HashMap::from([(41, AttachmentTaskControl { task, control })]);
            let (sink, queue) = crate::event_queue::event_queue(2).unwrap();
            ready_rx.await.unwrap();
            tokio::task::yield_now().await;

            abandon_timed_out_attachments(&sink, &mut tasks, &mut controls);

            assert!(tasks.is_empty());
            assert!(controls.is_empty());
            let crate::event_queue::EventPoll::Event(event) = queue.poll() else {
                panic!("expected one attachment interruption event");
            };
            assert_eq!(event.kind, crate::event::EVENT_ERROR);
            assert_eq!(event.request_id, 41);
            assert!(matches!(queue.poll(), crate::event_queue::EventPoll::Empty));
            assert!(tasks.join_next().await.is_none());
        });
    }

    #[test]
    fn deferred_failure_does_not_report_a_cancelled_attachment() {
        let admission = AttachmentAdmission::for_test(16, 1);
        let permit = admission.try_reserve(41, 1).unwrap();
        assert!(admission.cancel(41));

        assert!(
            deferred_command_failure(
                Command::SendAttachment {
                    request_id: 41,
                    recipient: "recipient".into(),
                    filename: "attachment.txt".into(),
                    content_type: "text/plain".into(),
                    data: vec![1],
                    group: false,
                    permit,
                },
                "recovery stopped",
            )
            .is_none()
        );
    }

    #[test]
    fn fails_deferred_requests_but_drops_ephemeral_typing() {
        let send = deferred_command_failure(
            Command::SendGroupMessage {
                request_id: 41,
                group_key: "group".into(),
                message: "message".into(),
            },
            "recovery stopped",
        )
        .unwrap();
        let leave = deferred_command_failure(
            Command::LeaveGroup {
                request_id: 42,
                group_key: "group".into(),
            },
            "recovery stopped",
        )
        .unwrap();
        let typing = deferred_command_failure(
            Command::SetTyping {
                request_id: 43,
                recipient: "recipient".into(),
                typing: true,
            },
            "recovery stopped",
        );

        assert_eq!(send.request_id, 41);
        assert_eq!(send.text.as_deref(), Some("recovery stopped"));
        assert_eq!(leave.request_id, 42);
        assert_eq!(leave.chat_id.as_deref(), Some("group"));
        assert!(typing.is_none());
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
    fn retries_closed_websocket_receipts_after_recovery_without_an_error_event() {
        let recipient = parse_recipient("11111111-1111-4111-8111-111111111111").unwrap();
        let mut receipts = DeliveryReceiptQueue::default();
        assert!(!receipts.enqueue(recipient, 42, 100));
        assert!(!receipts.enqueue(recipient, 42, 101));
        assert_eq!(receipts.len(), 1);
        assert!(receipts.start_next(false).is_none());
        let receipt = receipts.start_next(true).unwrap();
        let completion = DeliveryReceiptCompletion {
            generation: 7,
            receipt: receipt.clone(),
            result: Err(presage::Error::MessageSenderError(Box::new(
                MessageSenderError::ServiceError(ServiceError::WsClosing {
                    reason: "test close while waiting for a response",
                }),
            ))),
        };
        let (sink, events) = crate::event_queue::event_queue(2).unwrap();

        assert!(finish_delivery_receipt_attempt(&mut receipts, &sink, &completion).is_some());
        assert_eq!(receipts.len(), 1);
        assert!(matches!(
            events.poll(),
            crate::event_queue::EventPoll::Empty
        ));
        assert!(receipts.start_next(false).is_none());
        let retry = receipts.start_next(true).unwrap();
        assert_eq!(retry.identity, receipt.identity);
        assert_eq!(retry.send_timestamp, 100);
    }

    #[test]
    fn bounds_delivery_receipt_memory_and_reports_each_saturation_episode_once() {
        let recipient = parse_recipient("11111111-1111-4111-8111-111111111111").unwrap();
        let mut receipts = DeliveryReceiptQueue::default();

        for timestamp in 1..=MAX_PENDING_DELIVERY_RECEIPTS as u64 {
            assert!(!receipts.enqueue(recipient, timestamp, timestamp + 10_000));
        }
        assert_eq!(receipts.len(), MAX_PENDING_DELIVERY_RECEIPTS);
        assert!(receipts.enqueue(recipient, 20_000, 30_000));
        assert!(!receipts.enqueue(recipient, 20_001, 30_001));
        assert_eq!(receipts.len(), MAX_PENDING_DELIVERY_RECEIPTS);

        let completed = receipts.start_next(true).unwrap();
        receipts.complete(&completed.identity);
        assert!(!receipts.enqueue(recipient, 20_002, 30_002));
        assert_eq!(receipts.len(), MAX_PENDING_DELIVERY_RECEIPTS);
        assert!(receipts.enqueue(recipient, 20_003, 30_003));
    }

    #[test]
    fn defers_transient_receipt_failures_without_resetting_a_healthy_socket() {
        let recipient = parse_recipient("11111111-1111-4111-8111-111111111111").unwrap();
        let mut receipts = DeliveryReceiptQueue::default();
        assert!(!receipts.enqueue(recipient, 42, 100));
        let receipt = receipts.start_next(true).unwrap();
        let completion = DeliveryReceiptCompletion {
            generation: 7,
            receipt,
            result: Err(presage::Error::ServiceError(
                ServiceError::RateLimitExceeded { retry_after: None },
            )),
        };
        let (sink, events) = crate::event_queue::event_queue(2).unwrap();

        assert_eq!(
            finish_delivery_receipt_attempt(&mut receipts, &sink, &completion),
            None
        );
        assert!(receipts.start_next(true).is_none());
        assert!(matches!(
            events.poll(),
            crate::event_queue::EventPoll::Empty
        ));
        receipts.activate_retries();
        assert!(receipts.start_next(true).is_some());
    }

    #[test]
    fn reports_and_discards_permanent_delivery_receipt_failures() {
        let recipient = parse_recipient("11111111-1111-4111-8111-111111111111").unwrap();
        let mut receipts = DeliveryReceiptQueue::default();
        assert!(!receipts.enqueue(recipient, 42, 100));
        let receipt = receipts.start_next(true).unwrap();
        let completion = DeliveryReceiptCompletion {
            generation: 7,
            receipt,
            result: Err(presage::Error::ServiceError(ServiceError::Unauthorized)),
        };
        let (sink, events) = crate::event_queue::event_queue(2).unwrap();

        assert_eq!(
            finish_delivery_receipt_attempt(&mut receipts, &sink, &completion),
            None
        );
        assert_eq!(receipts.len(), 0);
        let crate::event_queue::EventPoll::Event(event) = events.poll() else {
            panic!("expected a permanent delivery receipt error");
        };
        assert_eq!(event.kind, crate::event::EVENT_ERROR);
        assert_eq!(event.flags, 0);
        assert!(matches!(
            events.poll(),
            crate::event_queue::EventPoll::Empty
        ));
    }

    #[test]
    fn aborts_owned_delivery_receipt_work_and_releases_it_for_retry() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        runtime.block_on(local.run_until(async {
            let recipient = parse_recipient("11111111-1111-4111-8111-111111111111").unwrap();
            let mut receipts = DeliveryReceiptQueue::default();
            assert!(!receipts.enqueue(recipient, 42, 100));
            let original = receipts.start_next(true).unwrap();
            let dropped = Arc::new(AtomicBool::new(false));
            let task_dropped = Arc::clone(&dropped);
            let (started_tx, started_rx) = oneshot::channel();
            let mut tasks = tokio::task::JoinSet::new();
            tasks.spawn_local(async move {
                let _drop_flag = DropFlag(task_dropped);
                let _ = started_tx.send(());
                std::future::pending::<DeliveryReceiptCompletion>().await
            });
            started_rx.await.unwrap();

            abort_delivery_receipt_tasks(&mut tasks, &mut receipts).await;

            assert!(tasks.is_empty());
            assert!(dropped.load(Ordering::Acquire));
            let retry = receipts.start_next(true).unwrap();
            assert_eq!(retry.identity, original.identity);
            assert_eq!(retry.send_timestamp, original.send_timestamp);
        }));
    }

    #[test]
    fn suppresses_pending_and_recently_completed_projection_identities() {
        let identity = ProjectionIdentity {
            sender: "aci:sender".into(),
            destination: "aci:destination".into(),
            timestamp_ms: 42,
        };
        let mut identities = ProjectionIdentities::default();

        assert!(identities.reserve(identity.clone()));
        assert!(!identities.reserve(identity.clone()));
        identities.release_pending(&identity);
        assert!(identities.reserve(identity.clone()));
        identities.complete(identity.clone());
        assert!(!identities.reserve(identity));
    }

    #[test]
    fn bounds_completed_projection_identity_memory() {
        let mut identities = ProjectionIdentities::default();

        for timestamp_ms in 0..=RECENT_PROJECTION_IDENTITY_LIMIT as i64 {
            let identity = ProjectionIdentity {
                sender: "aci:sender".into(),
                destination: "aci:destination".into(),
                timestamp_ms,
            };
            assert!(identities.reserve(identity.clone()));
            identities.complete(identity);
        }

        assert_eq!(identities.completed.len(), RECENT_PROJECTION_IDENTITY_LIMIT);
        assert!(identities.reserve(ProjectionIdentity {
            sender: "aci:sender".into(),
            destination: "aci:destination".into(),
            timestamp_ms: 0,
        }));
        assert!(!identities.reserve(ProjectionIdentity {
            sender: "aci:sender".into(),
            destination: "aci:destination".into(),
            timestamp_ms: RECENT_PROJECTION_IDENTITY_LIMIT as i64,
        }));
    }

    #[test]
    fn bounds_pending_message_projection_memory_and_reopens_capacity() {
        let acknowledgments = AcknowledgmentInbox::new();
        let mut projection = MessageProjection::new(Arc::clone(&acknowledgments));
        let mut delivery_ids = Vec::new();
        for timestamp_ms in 1..=MAX_PENDING_MESSAGE_PROJECTIONS as u64 {
            delivery_ids.push(
                projection
                    .track(projection_test_content(timestamp_ms, false))
                    .unwrap(),
            );
        }

        assert_eq!(projection.pending.len(), MAX_PENDING_MESSAGE_PROJECTIONS);
        assert!(!projection.has_capacity());
        assert!(
            projection
                .track(projection_test_content(
                    MAX_PENDING_MESSAGE_PROJECTIONS as u64 + 1,
                    false,
                ))
                .is_none()
        );
        projection.release(delivery_ids[0]).unwrap();
        assert!(projection.has_capacity());
        assert!(
            projection
                .track(projection_test_content(
                    MAX_PENDING_MESSAGE_PROJECTIONS as u64 + 1,
                    false,
                ))
                .is_some()
        );
        assert_eq!(projection.pending.len(), MAX_PENDING_MESSAGE_PROJECTIONS);
    }

    #[test]
    fn replay_queue_preserves_ready_order_and_defers_groups_until_authoritative() {
        let mut replay = MessageReplayQueue::default();
        replay.replace(
            vec![
                projection_test_content(1, false),
                projection_test_content(2, true),
                projection_test_content(3, false),
            ],
            false,
        );

        assert_eq!(content_timestamp(&replay.pop_ready().unwrap()), 1);
        assert_eq!(content_timestamp(&replay.pop_ready().unwrap()), 3);
        assert!(replay.pop_ready().is_none());
        replay.activate_groups();
        assert_eq!(content_timestamp(&replay.pop_ready().unwrap()), 2);
        assert!(replay.pop_ready().is_none());

        for timestamp_ms in 1..=MAX_PENDING_MESSAGE_PROJECTIONS as u64 {
            replay.push(projection_test_content(timestamp_ms, true), false);
        }
        assert!(!replay.can_accept_live_message());
        replay.activate_groups();
        replay.pop_ready().unwrap();
        assert!(replay.can_accept_live_message());
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
}
