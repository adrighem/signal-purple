// SPDX-License-Identifier: AGPL-3.0-only
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::{StreamExt, channel::oneshot, pin_mut};
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
    AttachmentPointer, EditMessage, ReceiptMessage, SyncMessage, TypingMessage, receipt_message,
    typing_message,
};
use presage::store::{ContentsStore, StateStore, Thread};
use presage::{Manager, manager::Registered};
use presage_store_sqlite::SqliteStore;
use presage_store_sqlite::{ClientOutboxKind, ClientOutboxMessage};
use qrcode::QrCode;
use qrcode::types::Color;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex as AsyncMutex, mpsc as tokio_mpsc, watch};
use zeroize::Zeroizing;

use crate::event::{
    EVENT_ATTACHMENT, EVENT_ATTACHMENT_SENT, EVENT_CONTACT, EVENT_CONTACT_SYNC_BEGIN,
    EVENT_CONTACT_SYNC_END, EVENT_DISCONNECTED, EVENT_GROUP, EVENT_GROUP_LEFT, EVENT_GROUP_MEMBER,
    EVENT_GROUP_MESSAGE, EVENT_GROUP_SYNC_BEGIN, EVENT_GROUP_SYNC_END, EVENT_IDENTITY_ACCEPTED,
    EVENT_IDENTITY_CHANGE, EVENT_LINK_QR, EVENT_MESSAGE, EVENT_READY, EVENT_RECEIPT,
    EVENT_RECOVERING, EVENT_TYPING, Event, FLAG_OUTGOING,
};

const MESSAGE_PROJECTION_CLIENT: &str = "signal-purple-v1";
const MAX_ATTACHMENT_BYTES: usize = 25 * 1024 * 1024;
const MAX_MESSAGE_ATTACHMENT_BYTES: usize = 50 * 1024 * 1024;
const MAX_QUEUED_EVENT_BYTES: usize = 64 * 1024 * 1024;
const GROUP_SYNC_RETRY_SECS: u64 = 30;
const RECOVERY_RETRY_DELAYS_SECS: [u64; 6] = [0, 1, 2, 4, 8, 16];
const RECENT_PROJECTION_IDENTITY_LIMIT: usize = 4096;

pub struct Config {
    pub store_path: String,
    pub device_name: String,
    pub passphrase: Zeroizing<String>,
}

pub(crate) struct WorkerContext {
    pub(crate) config: Config,
    pub(crate) commands: tokio_mpsc::Receiver<Command>,
    pub(crate) shutdown: watch::Receiver<bool>,
    pub(crate) events: mpsc::SyncSender<Event>,
    pub(crate) event_notification: Arc<EventNotification>,
    pub(crate) event_notification_writer: Arc<UnixStream>,
    pub(crate) overflowed: Arc<AtomicBool>,
    pub(crate) queued_bytes: Arc<AtomicUsize>,
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
    },
    CancelAttachment {
        request_id: u64,
    },
    SetTyping {
        request_id: u64,
        recipient: String,
        typing: bool,
    },
    AcknowledgeMessage {
        delivery_id: u64,
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

#[derive(Default)]
struct MessageProjection {
    next_delivery_id: u64,
    pending: HashMap<u64, Content>,
    identities: ProjectionIdentities,
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
    fn track(&mut self, content: Content) -> Option<u64> {
        if !self.identities.reserve(projection_identity(&content)) {
            return None;
        }
        self.next_delivery_id = self.next_delivery_id.wrapping_add(1).max(1);
        let delivery_id = self.next_delivery_id;
        self.pending.insert(delivery_id, content);
        Some(delivery_id)
    }

    fn release(&mut self, delivery_id: u64) -> Option<Content> {
        let content = self.pending.remove(&delivery_id)?;
        self.identities
            .release_pending(&projection_identity(&content));
        Some(content)
    }

    fn complete(&mut self, delivery_id: u64) -> Option<Content> {
        let content = self.pending.remove(&delivery_id)?;
        self.identities.complete(projection_identity(&content));
        Some(content)
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

#[derive(Clone)]
struct EventSink {
    sender: mpsc::SyncSender<Event>,
    notification: Arc<EventNotification>,
    notification_writer: Arc<UnixStream>,
    overflowed: Arc<AtomicBool>,
    queued_bytes: Arc<AtomicUsize>,
}

pub(crate) struct EventNotification {
    pending: AtomicBool,
    serial: Mutex<()>,
}

impl EventNotification {
    pub(crate) fn new() -> Self {
        Self {
            pending: AtomicBool::new(false),
            serial: Mutex::new(()),
        }
    }

    pub(crate) fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.serial
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn mark_pending(&self) -> bool {
        self.pending.swap(true, Ordering::AcqRel)
    }

    pub(crate) fn clear_pending(&self) {
        self.pending.store(false, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn is_pending(&self) -> bool {
        self.pending.load(Ordering::Acquire)
    }
}

impl EventSink {
    fn notify_locked(&self) {
        if self.notification.mark_pending() {
            return;
        }

        let mut writer = self.notification_writer.as_ref();
        loop {
            match writer.write(&[1]) {
                Ok(1) => return,
                Ok(_) => {
                    self.notification.clear_pending();
                    return;
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                // A full socket means its peer is already readable, so the
                // level-trigger invariant still holds.
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return,
                Err(_) => {
                    self.notification.clear_pending();
                    return;
                }
            }
        }
    }

    fn emit(&self, event: Event) {
        if self.overflowed.load(Ordering::Acquire) {
            return;
        }
        let event_bytes = event.data.len();
        if event_bytes > 0
            && self
                .queued_bytes
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |queued| {
                    queued
                        .checked_add(event_bytes)
                        .filter(|total| *total <= MAX_QUEUED_EVENT_BYTES)
                })
                .is_err()
        {
            let _notification_guard = self.notification.lock();
            self.overflowed.store(true, Ordering::Release);
            self.notify_locked();
            return;
        }
        let _notification_guard = self.notification.lock();
        match self.sender.try_send(event) {
            Ok(()) => self.notify_locked(),
            Err(error) => {
                if event_bytes > 0 {
                    self.queued_bytes.fetch_sub(event_bytes, Ordering::AcqRel);
                }
                if matches!(error, mpsc::TrySendError::Full(_)) {
                    self.overflowed.store(true, Ordering::Release);
                    self.notify_locked();
                }
            }
        }
    }
}

pub(crate) fn run_worker(context: WorkerContext) {
    let WorkerContext {
        config,
        commands,
        shutdown,
        events,
        event_notification,
        event_notification_writer,
        overflowed,
        queued_bytes,
        ready,
    } = context;
    let sink = EventSink {
        sender: events,
        notification: event_notification,
        notification_writer: event_notification_writer,
        overflowed,
        queued_bytes,
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?;
        let local = tokio::task::LocalSet::new();
        runtime.block_on(local.run_until(run(
            config,
            commands,
            shutdown,
            sink.clone(),
            Arc::clone(&ready),
        )))
    }));

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
    mut shutdown: watch::Receiver<bool>,
    sink: EventSink,
    ready: Arc<AtomicBool>,
) -> Result<(), String> {
    ensure_store_parent(&config.store_path)?;
    let open_store = SqliteStore::open_with_passphrase(
        &config.store_path,
        Some(config.passphrase.as_str()),
        OnNewIdentity::TrustUnverified,
    );
    pin_mut!(open_store);
    let store = tokio::select! {
        result = &mut open_store => {
            result.map_err(|error| {
                format!("Could not open encrypted Signal store: {error}")
            })?
        }
        _ = wait_for_shutdown(&mut shutdown) => return Ok(()),
    };

    let manager = if store.is_registered().await {
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
        match link_device(store, &config.device_name, &mut shutdown, &sink).await? {
            Some(manager) => manager,
            None => return Ok(()),
        }
    };

    receive_and_command_loop(manager, commands, shutdown, sink, ready).await
}

async fn wait_for_shutdown(shutdown: &mut watch::Receiver<bool>) {
    if *shutdown.borrow() {
        return;
    }
    let _ = shutdown.changed().await;
}

fn ensure_store_parent(store_path: &str) -> Result<(), String> {
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
    mut shutdown: watch::Receiver<bool>,
    sink: EventSink,
    ready: Arc<AtomicBool>,
) -> Result<(), String> {
    manager
        .store()
        .initialize_message_projection(MESSAGE_PROJECTION_CLIENT)
        .await
        .map_err(|error| format!("Could not initialize durable message replay: {error}"))?;
    manager
        .store()
        .initialize_identity_change_tracking()
        .await
        .map_err(|error| format!("Could not initialize identity-change tracking: {error}"))?;
    manager
        .store()
        .initialize_client_outbox()
        .await
        .map_err(|error| format!("Could not initialize the encrypted outbox: {error}"))?;
    let mut projection = MessageProjection::default();
    let mut deferred_commands = VecDeque::new();
    let mut attachment_tasks = tokio::task::JoinSet::new();
    let mut attachment_aborts = HashMap::new();
    let departed_groups = DepartedGroups::default();
    let mut retry_tick = tokio::time::interval(std::time::Duration::from_secs(5));
    retry_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut group_sync_retry_tick =
        tokio::time::interval(std::time::Duration::from_secs(GROUP_SYNC_RETRY_SECS));
    group_sync_retry_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    group_sync_retry_tick.reset();
    let mut recovery_backoff = RecoveryBackoff::default();
    let mut recovering = false;
    let mut last_recovery_error = None;

    loop {
        if recovering {
            if drain_recovery_commands(
                &mut manager,
                &mut commands,
                &mut shutdown,
                &sink,
                &mut projection,
                &departed_groups,
                &mut attachment_aborts,
                &mut deferred_commands,
            )
            .await
            {
                return Ok(());
            }
            let Some(delay) = recovery_backoff.next_delay() else {
                let error = last_recovery_error
                    .unwrap_or_else(|| "Signal message reception did not recover".into());
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
                                return Ok(());
                            };
                            if handle_recovery_command(
                                &mut manager,
                                command,
                                &mut shutdown,
                                &sink,
                                &mut projection,
                                &departed_groups,
                                &mut attachment_aborts,
                                &mut deferred_commands,
                            ).await {
                                return Ok(());
                            }
                        }
                        completed = attachment_tasks.join_next(),
                            if !attachment_tasks.is_empty() =>
                        {
                            if let Some(completed) = completed {
                                handle_attachment_completion(
                                    &manager,
                                    &sink,
                                    &mut attachment_aborts,
                                    completed,
                                ).await;
                            }
                        }
                        _ = wait_for_shutdown(&mut shutdown) => return Ok(()),
                    }
                }
            }
        }

        let messages = {
            let mut receive_manager = manager.clone();
            let mut receive = Box::pin(receive_manager.receive_messages());
            loop {
                tokio::select! {
                    result = &mut receive => break result,
                    command = commands.recv(), if recovering => {
                        let Some(command) = command else {
                            return Ok(());
                        };
                        if handle_recovery_command(
                            &mut manager,
                            command,
                            &mut shutdown,
                            &sink,
                            &mut projection,
                            &departed_groups,
                            &mut attachment_aborts,
                            &mut deferred_commands,
                        ).await {
                            return Ok(());
                        }
                    }
                    completed = attachment_tasks.join_next(),
                        if recovering && !attachment_tasks.is_empty() =>
                    {
                        if let Some(completed) = completed {
                            handle_attachment_completion(
                                &manager,
                                &sink,
                                &mut attachment_aborts,
                                completed,
                            ).await;
                        }
                    }
                    _ = wait_for_shutdown(&mut shutdown) => return Ok(()),
                }
            }
        };
        let messages = match messages {
            Ok(messages) => messages,
            Err(error) => {
                let transient = receive_error_is_transient(&error);
                let error = format!("Could not start Signal message reception: {error}");
                ready.store(false, Ordering::Release);
                if !transient {
                    fail_deferred_commands(
                        &sink,
                        &mut deferred_commands,
                        "Signal connection recovery stopped before the request could be sent",
                    );
                    return Err(error);
                }
                if !recovering {
                    sink.emit(Event {
                        kind: EVENT_RECOVERING,
                        ..Event::default()
                    });
                }
                let status = if recovery_backoff.has_remaining() {
                    "retrying automatically"
                } else {
                    "automatic retries exhausted"
                };
                sink.emit(Event::transient_error(format!("{error}; {status}")));
                last_recovery_error = Some(error);
                recovering = true;
                continue;
            }
        };
        pin_mut!(messages);

        let contact_sync = tokio::task::spawn_local(request_contacts_with_retries(
            manager.clone(),
            shutdown.clone(),
            sink.clone(),
        ));
        let mut synchronized = false;
        let mut groups_dirty = false;
        let mut groups_authoritative = false;

        loop {
            tokio::select! {
                received = messages.next() => {
                    match received {
                        Some(Received::QueueEmpty) => {
                            if !synchronized {
                                emit_contact_snapshot(&manager, &sink).await;
                                groups_authoritative =
                                    match synchronize_and_emit_group_snapshot(
                                        &mut manager,
                                        &sink,
                                        &departed_groups,
                                    ).await {
                                        Ok(()) => true,
                                        Err(error) => {
                                            sink.emit(Event::transient_error(error));
                                            group_sync_retry_tick.reset();
                                            false
                                        }
                                    };
                                replay_unprojected_messages(
                                    &mut manager,
                                    &sink,
                                    &mut projection,
                                    &departed_groups,
                                    groups_authoritative,
                                ).await;
                                emit_identity_changes(&manager, &sink).await;
                                retry_outbox(
                                    &mut manager,
                                    &sink,
                                    &departed_groups,
                                    groups_authoritative,
                                ).await;
                                groups_dirty = false;
                                synchronized = true;
                                recovering = false;
                                recovery_backoff.reset();
                                ready.store(true, Ordering::Release);
                                sink.emit(Event { kind: EVENT_READY, ..Event::default() });
                            } else if groups_dirty && groups_authoritative {
                                match emit_group_snapshot(
                                    &manager,
                                    &sink,
                                    &departed_groups,
                                ).await {
                                    Ok(()) => groups_dirty = false,
                                    Err(error) => {
                                        groups_authoritative = false;
                                        group_sync_retry_tick.reset();
                                        sink.emit(Event::transient_error(error));
                                    }
                                }
                            }
                        }
                        Some(Received::Contacts) => {
                            emit_contact_snapshot(&manager, &sink).await;
                        }
                        Some(Received::Content(content)) => {
                            groups_dirty |= content_has_group_context(&content.body);
                            if synchronized {
                                project_content(
                                    &mut manager,
                                    *content,
                                    &sink,
                                    &mut projection,
                                    &departed_groups,
                                    groups_authoritative,
                                ).await;
                                emit_identity_changes(&manager, &sink).await;
                            }
                        }
                        None => break,
                    }
                }
                command = async {
                    if synchronized
                        && let Some(command) = deferred_commands.pop_front()
                    {
                        Some(command)
                    } else {
                        commands.recv().await
                    }
                } => {
                    let Some(command) = command else {
                        contact_sync.abort();
                        return Ok(());
                    };
                    if !synchronized {
                        if handle_recovery_command(
                            &mut manager,
                            command,
                            &mut shutdown,
                            &sink,
                            &mut projection,
                            &departed_groups,
                            &mut attachment_aborts,
                            &mut deferred_commands,
                        ).await {
                            contact_sync.abort();
                            return Ok(());
                        }
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
                        } => {
                            if group && !groups_authoritative {
                                sink.emit(Event::request_error(
                                    request_id,
                                    "Signal groups are temporarily unavailable until authoritative synchronization succeeds",
                                ));
                                continue;
                            }
                            let mut attachment_manager = manager.clone();
                            let attachment_departed_groups = departed_groups.clone();
                            let abort = attachment_tasks.spawn_local(async move {
                                let result = upload_and_send_attachment(
                                    &mut attachment_manager,
                                    &recipient,
                                    filename,
                                    content_type,
                                    data,
                                    group,
                                    &attachment_departed_groups,
                                ).await;
                                (request_id, result)
                            });
                            attachment_aborts.insert(request_id, abort);
                        }
                        Command::CancelAttachment { request_id } => {
                            if let Some(abort) = attachment_aborts.remove(&request_id) {
                                abort.abort();
                            }
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
                                &mut projection,
                                &departed_groups,
                                groups_authoritative,
                            ).await {
                                contact_sync.abort();
                                return Ok(());
                            }
                            emit_identity_changes(&manager, &sink).await;
                        }
                    }
                }
                completed = attachment_tasks.join_next(), if !attachment_tasks.is_empty() => {
                    if let Some(completed) = completed {
                        handle_attachment_completion(
                            &manager,
                            &sink,
                            &mut attachment_aborts,
                            completed,
                        ).await;
                    }
                }
                _ = retry_tick.tick(), if synchronized => {
                    retry_outbox(
                        &mut manager,
                        &sink,
                        &departed_groups,
                        groups_authoritative,
                    ).await;
                }
                _ = group_sync_retry_tick.tick(), if synchronized && !groups_authoritative => {
                    match synchronize_and_emit_group_snapshot(
                        &mut manager,
                        &sink,
                        &departed_groups,
                    ).await {
                        Ok(()) => {
                            groups_authoritative = true;
                            groups_dirty = false;
                            replay_unprojected_messages(
                                &mut manager,
                                &sink,
                                &mut projection,
                                &departed_groups,
                                true,
                            ).await;
                            retry_outbox(&mut manager, &sink, &departed_groups, true).await;
                        }
                        Err(error) => sink.emit(Event::transient_error(error)),
                    }
                }
                _ = wait_for_shutdown(&mut shutdown) => {
                    contact_sync.abort();
                    return Ok(());
                },
            }
        }

        ready.store(false, Ordering::Release);
        if !recovering {
            sink.emit(Event {
                kind: EVENT_RECOVERING,
                ..Event::default()
            });
        }
        recovering = true;
        contact_sync.abort();
        let _ = contact_sync.await;
        abort_in_flight_attachments(
            &manager,
            &sink,
            &mut attachment_tasks,
            &mut attachment_aborts,
        )
        .await;
        let error = "Signal's message stream ended unexpectedly".to_owned();
        last_recovery_error = Some(error.clone());
        sink.emit(Event::transient_error(format!(
            "{error}; reconnecting automatically"
        )));
    }
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
    attachment_aborts: &mut HashMap<u64, tokio::task::AbortHandle>,
    completed: Result<(u64, Result<SentMessage, String>), tokio::task::JoinError>,
) {
    let Ok((request_id, result)) = completed else {
        return;
    };
    attachment_aborts.remove(&request_id);
    match result {
        Ok(sent) => {
            mark_sent_message_projected_or_report(manager, &sent, sink).await;
            sink.emit(Event {
                kind: EVENT_ATTACHMENT_SENT,
                request_id,
                ..Event::default()
            });
        }
        Err(error) => sink.emit(Event::request_error(request_id, error)),
    }
}

async fn abort_in_flight_attachments(
    manager: &Manager<SqliteStore, Registered>,
    sink: &EventSink,
    attachment_tasks: &mut tokio::task::JoinSet<(u64, Result<SentMessage, String>)>,
    attachment_aborts: &mut HashMap<u64, tokio::task::AbortHandle>,
) {
    let completions = abort_and_drain_tasks(attachment_tasks, attachment_aborts.values()).await;
    for completed in completions {
        handle_attachment_completion(manager, sink, attachment_aborts, completed).await;
    }
    for (request_id, _) in attachment_aborts.drain() {
        sink.emit(Event::request_error(
            request_id,
            "Signal connection was interrupted before the attachment completed",
        ));
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

fn cancel_deferred_attachment(commands: &mut VecDeque<Command>, request_id: u64) -> bool {
    let Some(index) = commands.iter().position(|command| {
        matches!(
            command,
            Command::SendAttachment {
                request_id: queued_request_id,
                ..
            } if *queued_request_id == request_id
        )
    }) else {
        return false;
    };
    commands.remove(index);
    true
}

fn deferred_command_failure(command: Command, message: &str) -> Option<Event> {
    match command {
        Command::LeaveGroup {
            request_id,
            group_key,
        } => Some(Event::group_request_error(request_id, group_key, message)),
        Command::SendMessage { request_id, .. }
        | Command::SendGroupMessage { request_id, .. }
        | Command::SendAttachment { request_id, .. }
        | Command::AcceptIdentity { request_id, .. }
        | Command::DismissIdentity { request_id, .. }
        | Command::MarkRead { request_id, .. } => Some(Event::request_error(request_id, message)),
        Command::CancelAttachment { .. }
        | Command::SetTyping { .. }
        | Command::AcknowledgeMessage { .. } => None,
    }
}

fn fail_deferred_commands(sink: &EventSink, commands: &mut VecDeque<Command>, message: &str) {
    while let Some(command) = commands.pop_front() {
        if let Some(event) = deferred_command_failure(command, message) {
            sink.emit(event);
        }
    }
}

async fn handle_recovery_command(
    manager: &mut Manager<SqliteStore, Registered>,
    command: Command,
    shutdown: &mut watch::Receiver<bool>,
    sink: &EventSink,
    projection: &mut MessageProjection,
    departed_groups: &DepartedGroups,
    attachment_aborts: &mut HashMap<u64, tokio::task::AbortHandle>,
    deferred_commands: &mut VecDeque<Command>,
) -> bool {
    match command {
        Command::AcknowledgeMessage { .. } => {
            handle_command_interruptibly(
                manager,
                command,
                shutdown,
                sink,
                projection,
                departed_groups,
                false,
            )
            .await
        }
        Command::CancelAttachment { request_id } => {
            cancel_deferred_attachment(deferred_commands, request_id);
            if let Some(abort) = attachment_aborts.remove(&request_id) {
                abort.abort();
            }
            false
        }
        Command::SetTyping { .. } => false,
        command => {
            deferred_commands.push_back(command);
            false
        }
    }
}

async fn drain_recovery_commands(
    manager: &mut Manager<SqliteStore, Registered>,
    commands: &mut tokio_mpsc::Receiver<Command>,
    shutdown: &mut watch::Receiver<bool>,
    sink: &EventSink,
    projection: &mut MessageProjection,
    departed_groups: &DepartedGroups,
    attachment_aborts: &mut HashMap<u64, tokio::task::AbortHandle>,
    deferred_commands: &mut VecDeque<Command>,
) -> bool {
    loop {
        match commands.try_recv() {
            Ok(command) => {
                if handle_recovery_command(
                    manager,
                    command,
                    shutdown,
                    sink,
                    projection,
                    departed_groups,
                    attachment_aborts,
                    deferred_commands,
                )
                .await
                {
                    return true;
                }
            }
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
    projection: &mut MessageProjection,
    departed_groups: &DepartedGroups,
    groups_authoritative: bool,
) -> bool {
    let operation = handle_command(
        manager,
        command,
        sink,
        projection,
        departed_groups,
        groups_authoritative,
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
                for contact in contacts {
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
    for (key, group) in groups {
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
        for member in group.members {
            sink.emit(Event {
                kind: EVENT_GROUP_MEMBER,
                chat_id: Some(chat_id.clone()),
                peer_id: Some(ServiceId::Aci(member.aci).service_id_string()),
                value: i32::from(member.role == Role::Administrator),
                ..Event::default()
            });
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
            for change in changes {
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
                    now_ms().saturating_add(retry_delay_ms(attempts)),
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
    let messages = match manager.store().due_client_messages(now_ms()).await {
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
) -> Result<(), String> {
    let timestamp = now_ms();
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

async fn upload_and_send_attachment(
    manager: &mut Manager<SqliteStore, Registered>,
    recipient: &str,
    filename: String,
    content_type: String,
    data: Vec<u8>,
    group: bool,
    departed_groups: &DepartedGroups,
) -> Result<SentMessage, String> {
    if data.is_empty() || data.len() > MAX_ATTACHMENT_BYTES {
        return Err("Attachment size is outside the supported range".into());
    }
    let group_target = if group {
        Some(
            resolve_active_group(manager, recipient, departed_groups)
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
    let timestamp = now_ms();
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
        let recipient = parse_recipient(recipient)
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

async fn replay_unprojected_messages(
    manager: &mut Manager<SqliteStore, Registered>,
    sink: &EventSink,
    projection: &mut MessageProjection,
    departed_groups: &DepartedGroups,
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

    for content in messages {
        project_content(
            manager,
            content,
            sink,
            projection,
            departed_groups,
            groups_authoritative,
        )
        .await;
    }
}

async fn project_content(
    manager: &mut Manager<SqliteStore, Registered>,
    content: Content,
    sink: &EventSink,
    projection: &mut MessageProjection,
    departed_groups: &DepartedGroups,
    groups_authoritative: bool,
) {
    if !content_is_projectable(&content.body, groups_authoritative) {
        return;
    }
    let Some(delivery_id) = projection.track(content.clone()) else {
        return;
    };
    let effect = projection_effect(
        handle_content(manager, content.clone(), delivery_id, sink, departed_groups).await,
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

async fn handle_command(
    manager: &mut Manager<SqliteStore, Registered>,
    command: Command,
    sink: &EventSink,
    projection: &mut MessageProjection,
    departed_groups: &DepartedGroups,
    groups_authoritative: bool,
) {
    if let Command::AcknowledgeMessage { delivery_id } = command {
        let Some(content) = projection.pending.get(&delivery_id) else {
            return;
        };
        match manager
            .store()
            .mark_message_projected(MESSAGE_PROJECTION_CLIENT, content)
            .await
        {
            Ok(()) => {
                projection.complete(delivery_id);
            }
            Err(error) => {
                projection.release(delivery_id);
                sink.emit(Event::error(
                    format!("Could not acknowledge a displayed Signal message: {error}"),
                    false,
                ));
            }
        }
        return;
    }

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
            Some(recipient) => {
                send_receipt(manager, recipient, timestamp, receipt_message::Type::Read)
                    .await
                    .map_err(|error| error.to_string())
            }
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
                    let timestamp = now_ms();
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
        Command::SendAttachment { .. } | Command::CancelAttachment { .. } => unreachable!(),
        Command::LeaveGroup { .. } => unreachable!(),
        Command::AcknowledgeMessage { .. } => unreachable!(),
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
            if content.metadata.needs_receipt {
                let mut receipt_manager = manager.clone();
                let receipt_sink = sink.clone();
                let receipt_recipient = content.metadata.sender;
                tokio::task::spawn_local(async move {
                    send_delivery_receipt(
                        &mut receipt_manager,
                        receipt_recipient,
                        timestamp,
                        &receipt_sink,
                    )
                    .await;
                });
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

fn inline_group_image_matches(content_type: Option<&str>, data: &[u8]) -> bool {
    match content_type {
        Some(content_type) if content_type.eq_ignore_ascii_case("image/jpeg") => {
            data.starts_with(&[0xff, 0xd8, 0xff])
        }
        Some(content_type) if content_type.eq_ignore_ascii_case("image/png") => {
            data.starts_with(b"\x89PNG\r\n\x1a\n")
        }
        _ => false,
    }
}

fn should_inline_group_image(
    outgoing: bool,
    group: bool,
    content_type: Option<&str>,
    data: Option<&[u8]>,
) -> bool {
    !outgoing && group && data.is_some_and(|data| inline_group_image_matches(content_type, data))
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
    let mut downloaded_bytes = 0usize;
    if !outgoing {
        for (attachment_index, attachment) in attachments.iter().enumerate() {
            let declared_size = attachment.size.unwrap_or_default() as usize;
            if declared_size > MAX_ATTACHMENT_BYTES
                || downloaded_bytes.saturating_add(declared_size) > MAX_MESSAGE_ATTACHMENT_BYTES
            {
                sink.emit(Event::error(
                    format!(
                        "Rejected Signal attachment larger than the configured {} MiB limit",
                        MAX_ATTACHMENT_BYTES / (1024 * 1024)
                    ),
                    false,
                ));
                continue;
            }
            match manager.get_attachment(attachment).await {
                Ok(data) if data.is_empty() => sink.emit(Event::error(
                    "Could not download a Signal attachment: decrypted attachment was empty",
                    false,
                )),
                Ok(data)
                    if data.len() <= MAX_ATTACHMENT_BYTES
                        && downloaded_bytes.saturating_add(data.len())
                            <= MAX_MESSAGE_ATTACHMENT_BYTES =>
                {
                    downloaded_bytes += data.len();
                    downloaded.push((attachment_index, attachment, data));
                }
                Ok(_) => sink.emit(Event::error(
                    "Rejected a Signal attachment which exceeded its size limit after decryption",
                    false,
                )),
                Err(error) => sink.emit(Event::error(
                    format!("Could not download a Signal attachment: {error}"),
                    false,
                )),
            }
        }
    }

    let inline_attachment_indexes: HashSet<usize> = downloaded
        .iter()
        .filter_map(|(attachment_index, attachment, data)| {
            should_inline_group_image(
                outgoing,
                group_key.is_some(),
                attachment.content_type.as_deref(),
                Some(data),
            )
            .then_some(*attachment_index)
        })
        .collect();
    let text = projected_data_message_text(
        text,
        attachments
            .iter()
            .enumerate()
            .map(|(attachment_index, attachment)| {
                (
                    attachment.file_name.as_deref(),
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
    for (index, (_, attachment, data)) in downloaded.into_iter().enumerate() {
        sink.emit(Event {
            kind: EVENT_ATTACHMENT,
            request_id: if index + 1 == attachment_count {
                delivery_id
            } else {
                0
            },
            peer_id: Some(peer.to_owned()),
            chat_id: group_key.map(|key| group_identifier(&key)),
            title: attachment
                .file_name
                .clone()
                .or_else(|| Some("Signal attachment".into())),
            text: attachment.content_type.clone(),
            data,
            timestamp_ms: timestamp,
            ..Event::default()
        });
    }
    ProjectionDisposition::AwaitingAck
}

async fn send_delivery_receipt(
    manager: &mut Manager<SqliteStore, Registered>,
    recipient: ServiceId,
    message_timestamp: u64,
    sink: &EventSink,
) {
    if let Err(error) = send_receipt(
        manager,
        recipient,
        message_timestamp,
        receipt_message::Type::Delivery,
    )
    .await
    {
        sink.emit(Event::error(
            format!("Could not send a Signal delivery receipt: {error}"),
            false,
        ));
    }
}

async fn send_receipt(
    manager: &mut Manager<SqliteStore, Registered>,
    recipient: ServiceId,
    message_timestamp: u64,
    receipt_type: receipt_message::Type,
) -> Result<(), presage::Error<presage_store_sqlite::SqliteStoreError>> {
    let timestamp = now_ms();
    manager
        .send_message(
            recipient,
            ReceiptMessage {
                r#type: Some(receipt_type.into()),
                timestamp: vec![message_timestamp],
            },
            timestamp,
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

fn now_ms() -> u64 {
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
    use std::io::Read;

    fn test_notification() -> (Arc<EventNotification>, Arc<UnixStream>, UnixStream) {
        let (reader, writer) = UnixStream::pair().unwrap();
        reader.set_nonblocking(true).unwrap();
        writer.set_nonblocking(true).unwrap();
        (Arc::new(EventNotification::new()), Arc::new(writer), reader)
    }

    #[test]
    fn derives_stable_non_secret_group_identifiers() {
        let first = group_identifier(&[0; 32]);
        let second = group_identifier(&[1; 32]);

        assert_eq!(first.len(), 64);
        assert_eq!(first, group_identifier(&[0; 32]));
        assert_ne!(first, hex::encode([0; 32]));
        assert_ne!(first, second);
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

    #[test]
    fn recognizes_only_declared_jpeg_and_png_payloads_for_inline_display() {
        let jpeg = [0xff, 0xd8, 0xff, 0xe0];
        let png = b"\x89PNG\r\n\x1a\nrest";

        assert!(inline_group_image_matches(Some("image/jpeg"), &jpeg));
        assert!(inline_group_image_matches(Some("IMAGE/JPEG"), &jpeg));
        assert!(inline_group_image_matches(Some("image/png"), png));
        assert!(inline_group_image_matches(Some("IMAGE/PNG"), png));

        assert!(!inline_group_image_matches(Some("image/png"), &jpeg));
        assert!(!inline_group_image_matches(Some("image/jpeg"), png));
        assert!(!inline_group_image_matches(Some("image/png"), b"\x89PNG"));
        assert!(!inline_group_image_matches(Some("image/gif"), b"GIF89a"));
        assert!(!inline_group_image_matches(
            Some("image/jpeg; charset=binary"),
            &jpeg
        ));
        assert!(!inline_group_image_matches(None, &jpeg));
    }

    #[test]
    fn inlines_only_downloaded_incoming_group_images() {
        let jpeg = [0xff, 0xd8, 0xff, 0xe0];

        assert!(should_inline_group_image(
            false,
            true,
            Some("image/jpeg"),
            Some(&jpeg)
        ));
        assert!(!should_inline_group_image(
            false,
            false,
            Some("image/jpeg"),
            Some(&jpeg)
        ));
        assert!(!should_inline_group_image(
            true,
            true,
            Some("image/jpeg"),
            Some(&jpeg)
        ));
        assert!(!should_inline_group_image(
            false,
            true,
            Some("application/octet-stream"),
            Some(&jpeg)
        ));
        assert!(!should_inline_group_image(
            false,
            true,
            Some("image/jpeg"),
            None
        ));
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
    fn cancels_an_attachment_before_deferred_replay() {
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
            },
        ]);

        assert!(cancel_deferred_attachment(&mut commands, 41));
        assert!(!cancel_deferred_attachment(&mut commands, 41));
        assert!(matches!(
            commands.pop_front(),
            Some(Command::SendMessage { request_id: 40, .. })
        ));
        assert!(commands.is_empty());
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
        let relink = presage::Error::<presage_store_sqlite::SqliteStoreError>::RelinkNecessary;

        assert!(receive_error_is_transient(&websocket_closing));
        assert!(receive_error_is_transient(&rate_limited));
        assert!(receive_error_is_transient(&websocket_unavailable));
        assert!(!receive_error_is_transient(&unauthorized));
        assert!(!receive_error_is_transient(&websocket_unauthorized));
        assert!(!receive_error_is_transient(&relink));
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
    fn reports_event_queue_overflow_without_growing_the_queue() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let (notification, notification_writer, mut notification_reader) = test_notification();
        let overflowed = Arc::new(AtomicBool::new(false));
        let sink = EventSink {
            sender,
            notification: Arc::clone(&notification),
            notification_writer,
            overflowed: Arc::clone(&overflowed),
            queued_bytes: Arc::new(AtomicUsize::new(0)),
        };

        sink.emit(Event {
            kind: EVENT_MESSAGE,
            ..Event::default()
        });
        sink.emit(Event {
            kind: EVENT_GROUP_MESSAGE,
            ..Event::default()
        });
        sink.emit(Event::default());

        assert!(overflowed.load(Ordering::Acquire));
        assert!(notification.is_pending());
        let mut token = [0u8; 1];
        assert_eq!(notification_reader.read(&mut token).unwrap(), 1);
        assert_eq!(token, [1]);
        assert_eq!(
            notification_reader.read(&mut token).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert_eq!(receiver.try_recv().unwrap().kind, EVENT_MESSAGE);
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
    }

    #[test]
    fn bounds_binary_event_queue_memory() {
        let (sender, receiver) = mpsc::sync_channel(4);
        let (notification, notification_writer, _notification_reader) = test_notification();
        let overflowed = Arc::new(AtomicBool::new(false));
        let queued_bytes = Arc::new(AtomicUsize::new(0));
        let sink = EventSink {
            sender,
            notification,
            notification_writer,
            overflowed: Arc::clone(&overflowed),
            queued_bytes: Arc::clone(&queued_bytes),
        };

        sink.emit(Event {
            kind: EVENT_ATTACHMENT,
            data: vec![0; MAX_QUEUED_EVENT_BYTES],
            ..Event::default()
        });
        sink.emit(Event {
            kind: EVENT_ATTACHMENT,
            data: vec![0],
            ..Event::default()
        });

        assert!(overflowed.load(Ordering::Acquire));
        assert_eq!(queued_bytes.load(Ordering::Acquire), MAX_QUEUED_EVENT_BYTES);
        assert_eq!(
            receiver.try_recv().unwrap().data.len(),
            MAX_QUEUED_EVENT_BYTES
        );
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
    }

    #[test]
    fn notifies_when_binary_overflow_happens_before_any_enqueue() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let (notification, notification_writer, mut notification_reader) = test_notification();
        let overflowed = Arc::new(AtomicBool::new(false));
        let sink = EventSink {
            sender,
            notification: Arc::clone(&notification),
            notification_writer,
            overflowed: Arc::clone(&overflowed),
            queued_bytes: Arc::new(AtomicUsize::new(0)),
        };

        sink.emit(Event {
            kind: EVENT_ATTACHMENT,
            data: vec![0; MAX_QUEUED_EVENT_BYTES + 1],
            ..Event::default()
        });

        assert!(overflowed.load(Ordering::Acquire));
        assert!(notification.is_pending());
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        let mut token = [0u8; 1];
        assert_eq!(notification_reader.read(&mut token).unwrap(), 1);
        assert_eq!(token, [1]);
    }

    #[test]
    fn failed_notification_write_does_not_leave_pending_set() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let (reader, writer) = UnixStream::pair().unwrap();
        writer.set_nonblocking(true).unwrap();
        drop(reader);
        let notification = Arc::new(EventNotification::new());
        let sink = EventSink {
            sender,
            notification: Arc::clone(&notification),
            notification_writer: Arc::new(writer),
            overflowed: Arc::new(AtomicBool::new(false)),
            queued_bytes: Arc::new(AtomicUsize::new(0)),
        };

        sink.emit(Event {
            kind: EVENT_MESSAGE,
            ..Event::default()
        });

        assert!(!notification.is_pending());
        assert_eq!(receiver.try_recv().unwrap().kind, EVENT_MESSAGE);
    }

    #[test]
    fn coalesces_notifications_until_the_event_queue_is_observed_empty() {
        let (sender, receiver) = mpsc::sync_channel(2);
        let (notification, notification_writer, mut notification_reader) = test_notification();
        let sink = EventSink {
            sender,
            notification: Arc::clone(&notification),
            notification_writer,
            overflowed: Arc::new(AtomicBool::new(false)),
            queued_bytes: Arc::new(AtomicUsize::new(0)),
        };
        let mut token = [0u8; 1];

        sink.emit(Event {
            kind: EVENT_MESSAGE,
            ..Event::default()
        });
        assert_eq!(receiver.try_recv().unwrap().kind, EVENT_MESSAGE);
        sink.emit(Event {
            kind: EVENT_GROUP_MESSAGE,
            ..Event::default()
        });

        assert_eq!(notification_reader.read(&mut token).unwrap(), 1);
        assert_eq!(
            notification_reader.read(&mut token).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert_eq!(receiver.try_recv().unwrap().kind, EVENT_GROUP_MESSAGE);

        {
            let _guard = notification.lock();
            notification.clear_pending();
        }
        sink.emit(Event {
            kind: EVENT_MESSAGE,
            ..Event::default()
        });
        assert_eq!(notification_reader.read(&mut token).unwrap(), 1);
    }
}
