// SPDX-License-Identifier: AGPL-3.0-only
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use presage::libsignal_service::content::{Content, ContentBody, EditMessage, SyncMessage};
use presage::libsignal_service::protocol::ServiceId;
use presage::proto::{ReceiptMessage, receipt_message};
use presage::{Manager, manager::Registered};
use presage_store_sqlite::SqliteStore;

use super::coordinator::{
    DepartedGroups, MessageTimestampAllocator, delivery_receipt_failure_action, handle_content,
    projection_effect, sqlite_store_error_is_transient,
};
use crate::acknowledgment::AcknowledgmentInbox;
use crate::event::Event;
use crate::event_queue::EventSink;
use crate::store::StorageRepository;

pub(crate) const MAX_PENDING_MESSAGE_PROJECTIONS: usize = 64;
pub(crate) const MAX_PENDING_DELIVERY_RECEIPTS: usize = 4096;
pub(crate) const RECENT_PROJECTION_IDENTITY_LIMIT: usize = 4096;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ProjectionIdentity {
    pub(crate) sender: String,
    pub(crate) destination: String,
    pub(crate) timestamp_ms: i64,
}

#[derive(Default)]
pub(crate) struct ProjectionIdentities {
    pub(crate) pending: HashSet<ProjectionIdentity>,
    pub(crate) completed: HashSet<ProjectionIdentity>,
    pub(crate) completed_order: VecDeque<ProjectionIdentity>,
}

impl ProjectionIdentities {
    pub(crate) fn reserve(&mut self, identity: ProjectionIdentity) -> bool {
        if self.completed.contains(&identity) {
            return false;
        }
        self.pending.insert(identity)
    }

    pub(crate) fn release_pending(&mut self, identity: &ProjectionIdentity) {
        self.pending.remove(identity);
    }

    pub(crate) fn complete(&mut self, identity: ProjectionIdentity) {
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

pub(crate) fn projection_identity(content: &Content) -> ProjectionIdentity {
    ProjectionIdentity {
        sender: content.metadata.sender.service_id_string(),
        destination: content.metadata.destination.service_id_string(),
        timestamp_ms: content.metadata.timestamp.timestamp_millis(),
    }
}

pub(crate) struct MessageProjection {
    pub(crate) next_delivery_id: u64,
    pub(crate) pending: HashMap<u64, Content>,
    pub(crate) identities: ProjectionIdentities,
    pub(crate) acknowledgments: Arc<AcknowledgmentInbox>,
    pub(crate) delivery_receipts: DeliveryReceiptQueue,
    pub(crate) delivery_receipt_tasks: tokio::task::JoinSet<DeliveryReceiptCompletion>,
}

impl MessageProjection {
    pub(crate) fn new(acknowledgments: Arc<AcknowledgmentInbox>) -> Self {
        Self {
            next_delivery_id: 0,
            pending: HashMap::new(),
            identities: ProjectionIdentities::default(),
            acknowledgments,
            delivery_receipts: DeliveryReceiptQueue::default(),
            delivery_receipt_tasks: tokio::task::JoinSet::new(),
        }
    }

    pub(crate) fn track(&mut self, content: Content) -> Option<u64> {
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

    pub(crate) fn has_capacity(&self) -> bool {
        self.pending.len() < MAX_PENDING_MESSAGE_PROJECTIONS
    }

    pub(crate) fn release(&mut self, delivery_id: u64) -> Option<Content> {
        let content = self.pending.remove(&delivery_id)?;
        self.acknowledgments.unregister(delivery_id);
        self.identities
            .release_pending(&projection_identity(&content));
        Some(content)
    }

    pub(crate) fn complete(&mut self, delivery_id: u64) -> Option<Content> {
        let content = self.pending.remove(&delivery_id)?;
        self.acknowledgments.unregister(delivery_id);
        self.identities.complete(projection_identity(&content));
        Some(content)
    }
}

#[derive(Default)]
pub(crate) struct MessageReplayQueue {
    pub(crate) ready: VecDeque<Content>,
    pub(crate) waiting_for_groups: VecDeque<Content>,
}

impl MessageReplayQueue {
    pub(crate) fn replace(&mut self, messages: Vec<Content>, groups_authoritative: bool) {
        self.ready.clear();
        self.waiting_for_groups.clear();
        for content in messages {
            self.push(content, groups_authoritative);
        }
    }

    pub(crate) fn push(&mut self, content: Content, groups_authoritative: bool) {
        if content_has_group_context(&content.body) && !groups_authoritative {
            self.waiting_for_groups.push_back(content);
        } else {
            self.ready.push_back(content);
        }
    }

    pub(crate) fn activate_groups(&mut self) {
        self.ready.append(&mut self.waiting_for_groups);
    }

    pub(crate) fn pop_ready(&mut self) -> Option<Content> {
        self.ready.pop_front()
    }

    pub(crate) fn can_accept_live_message(&self) -> bool {
        self.ready
            .len()
            .saturating_add(self.waiting_for_groups.len())
            < MAX_PENDING_MESSAGE_PROJECTIONS
    }
}

pub(crate) fn content_has_group_context(content: &ContentBody) -> bool {
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

pub(crate) fn content_is_projectable(content: &ContentBody, groups_authoritative: bool) -> bool {
    groups_authoritative || !content_has_group_context(content)
}

pub(crate) async fn project_content(
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
    let repo = StorageRepository::new(manager.store().clone());
    match repo.mark_message_projected(&content).await {
        Ok(()) => {
            projection.complete(delivery_id);
        }
        Err(error) => {
            projection.release(delivery_id);
            if sqlite_store_error_is_transient(&error) {
                tracing::warn!(%error, "Transient store contention recording handled Signal message; releasing for replay");
            } else {
                sink.emit(Event::error(
                    format!("Could not record a handled Signal message: {error}"),
                    false,
                ));
            }
        }
    }
}

pub(crate) async fn acknowledge_message(
    manager: &Manager<SqliteStore, Registered>,
    delivery_id: u64,
    sink: &EventSink,
    projection: &mut MessageProjection,
) -> bool {
    let Some(content) = projection.pending.get(&delivery_id) else {
        projection.acknowledgments.unregister(delivery_id);
        return true;
    };
    let repo = StorageRepository::new(manager.store().clone());
    match repo.mark_message_projected(content).await {
        Ok(()) => {
            projection.complete(delivery_id);
            true
        }
        Err(error) => {
            if sqlite_store_error_is_transient(&error) {
                tracing::warn!(%error, "Transient store contention acknowledging displayed Signal message; will retry");
            } else {
                sink.emit(Event::error(
                    format!("Could not acknowledge a displayed Signal message: {error}"),
                    false,
                ));
            }
            false
        }
    }
}

pub(crate) async fn process_acknowledgments(
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

pub(crate) async fn drain_acknowledgments(
    manager: &Manager<SqliteStore, Registered>,
    acknowledgments: &AcknowledgmentInbox,
    sink: &EventSink,
    projection: &mut MessageProjection,
) {
    acknowledgments.close();
    while process_acknowledgments(manager, acknowledgments, sink, projection, false).await != 0 {}
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct DeliveryReceiptIdentity {
    pub(crate) recipient: String,
    pub(crate) message_timestamp: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingDeliveryReceipt {
    pub(crate) identity: DeliveryReceiptIdentity,
    pub(crate) recipient: ServiceId,
    pub(crate) send_timestamp: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum DeliveryReceiptState {
    #[default]
    Ready,
    InFlight,
    Deferred,
}

#[derive(Default)]
pub(crate) struct DeliveryReceiptQueue {
    pub(crate) pending: VecDeque<(PendingDeliveryReceipt, DeliveryReceiptState)>,
    pub(crate) identities: HashSet<DeliveryReceiptIdentity>,
    pub(crate) limit_reported: bool,
}

impl DeliveryReceiptQueue {
    pub(crate) fn enqueue(
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

    pub(crate) fn start_next(&mut self, session_ready: bool) -> Option<PendingDeliveryReceipt> {
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

    pub(crate) fn complete(&mut self, identity: &DeliveryReceiptIdentity) {
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

    pub(crate) fn retry(&mut self, identity: &DeliveryReceiptIdentity, immediately: bool) {
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

    pub(crate) fn activate_retries(&mut self) {
        for (_, state) in &mut self.pending {
            if *state == DeliveryReceiptState::Deferred {
                *state = DeliveryReceiptState::Ready;
            }
        }
    }

    pub(crate) fn release_in_flight(&mut self) {
        for (_, state) in &mut self.pending {
            if *state == DeliveryReceiptState::InFlight {
                *state = DeliveryReceiptState::Ready;
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.pending.len()
    }
}

pub(crate) struct DeliveryReceiptCompletion {
    pub(crate) generation: u64,
    pub(crate) receipt: PendingDeliveryReceipt,
    pub(crate) result: Result<(), presage::Error<presage_store_sqlite::SqliteStoreError>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DeliveryReceiptFailureAction {
    Retry,
    Recover,
    Discard,
}

pub(crate) fn spawn_delivery_receipt_attempt(
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

pub(crate) fn finish_delivery_receipt_attempt(
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

pub(crate) async fn abort_delivery_receipt_tasks(
    tasks: &mut tokio::task::JoinSet<DeliveryReceiptCompletion>,
    receipts: &mut DeliveryReceiptQueue,
) {
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    receipts.release_in_flight();
}

pub(crate) async fn send_receipt_at_timestamp(
    manager: &mut Manager<SqliteStore, Registered>,
    recipient: ServiceId,
    message_timestamp: u64,
    receipt_type: receipt_message::Type,
    send_timestamp: u64,
) -> Result<(), presage::Error<presage_store_sqlite::SqliteStoreError>> {
    Box::pin(manager.send_message(
        recipient,
        ReceiptMessage {
            r#type: Some(receipt_type.into()),
            timestamp: vec![message_timestamp],
        },
        send_timestamp,
    ))
    .await
}

pub(crate) async fn send_receipt(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::coordinator::parse_recipient;
    use presage::libsignal_service::content::{
        ContentBody, DataMessage, GroupContextV2, ServiceError,
    };
    use presage::libsignal_service::sender::MessageSenderError;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, UNIX_EPOCH};
    use tokio::sync::oneshot;

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

    fn content_timestamp(content: &Content) -> u64 {
        content.metadata.timestamp.timestamp_millis() as u64
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
    fn finishes_delivery_receipt_retry_and_preserves_queue() {
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
}
