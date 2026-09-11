// SPDX-License-Identifier: AGPL-3.0-only
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::ops::ControlFlow;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::channel::oneshot;
use futures::{FutureExt, StreamExt, pin_mut};
use presage::libsignal_service::configuration::SignalServers;
use presage::model::identity::OnNewIdentity;
use presage::model::messages::Received;
use presage::store::StateStore;
use presage::{Manager, manager::Registered};
use presage_store_sqlite::SqliteStore;
use tokio::sync::{mpsc as tokio_mpsc, watch};

use super::command::{Command, Config, StorePassphrase, WorkerContext};
use super::coordinator::{
    ActiveReceiveTasks, AttachmentCompletion, AttachmentTaskControl, AttachmentTaskResult,
    CommandContext, DepartedGroups, GROUP_SYNC_RETRY_SECS, MessageTimestampAllocator,
    MetadataCache, OutgoingAttachment, RECEIVE_EVENT_QUEUE_CAPACITY, ReceiveStartError,
    RecoveryTransition, SHUTDOWN_CLEANUP_TIMEOUT, SentMessage, SessionState, emit_account_identity,
    emit_contact_snapshot, emit_group_snapshot, emit_identity_changes,
    fetch_missing_avatars_after_queue_drain, handle_attachment_completion, handle_command,
    load_unprojected_messages, qr_png, receive_error_is_transient,
    request_contacts_after_queue_drain, spawn_group_sync, upload_and_send_attachment,
};
use super::media::AvatarCache;
use super::outbox::retry_outbox;
use super::projection::{
    MessageProjection, MessageReplayQueue, abort_delivery_receipt_tasks, content_has_group_context,
    drain_acknowledgments, finish_delivery_receipt_attempt, process_acknowledgments,
    project_content, spawn_delivery_receipt_attempt,
};
use super::shutdown::{await_or_shutdown, finish_shutdown_cleanup, wait_for_shutdown};
use crate::acknowledgment::AcknowledgmentInbox;
#[cfg(test)]
use crate::attachment::AttachmentPayload;
use crate::attachment::AttachmentPermit;
use crate::event::{EVENT_LINK_QR, EVENT_READY, EVENT_RECOVERING, Event};
use crate::event_queue::EventSink;
use crate::store::StorageRepository;

pub fn run_worker(context: WorkerContext) {
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
            Box::pin(run(
                config,
                commands,
                worker_acknowledgments,
                shutdown,
                sink.clone(),
                Arc::clone(&ready),
            )),
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
    let avatar_cache = AvatarCache::new(Some(&store_path));
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

    Box::pin(receive_and_command_loop(
        manager,
        commands,
        acknowledgments,
        shutdown,
        sink,
        ready,
        avatar_cache,
    ))
    .await
}

pub(crate) async fn open_encrypted_store(
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

pub(crate) fn shutdown_runtime(runtime: tokio::runtime::Runtime, timeout: Duration) {
    runtime.shutdown_timeout(timeout);
}

pub(crate) fn run_local_future<F>(
    runtime: tokio::runtime::Runtime,
    local: tokio::task::LocalSet,
    mut future: Pin<Box<F>>,
    shutdown_timeout: Duration,
) -> std::thread::Result<F::Output>
where
    F: Future + ?Sized,
{
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        runtime.block_on(local.run_until(&mut future))
    }));
    drop(local);
    shutdown_runtime(runtime, shutdown_timeout);
    result
}

pub fn ensure_store_parent(store_path: &str) -> Result<(), String> {
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

/// Bundles the state that lives for the duration of one `run_worker` call
/// (across every reconnect) so it can be threaded through named phase methods
/// instead of as a long, repeated argument list.
struct ConnectionSession {
    manager: Manager<SqliteStore, Registered>,
    commands: tokio_mpsc::Receiver<Command>,
    acknowledgments: Arc<AcknowledgmentInbox>,
    shutdown: watch::Receiver<bool>,
    sink: EventSink,
    ready: Arc<AtomicBool>,
    avatar_cache: AvatarCache,
    repo: StorageRepository,
    timestamps: MessageTimestampAllocator,
    projection: MessageProjection,
    replay: MessageReplayQueue,
    deferred_commands: VecDeque<Command>,
    attachment_tasks: tokio::task::JoinSet<AttachmentCompletion>,
    attachment_aborts: HashMap<u64, AttachmentTaskControl>,
    receive_generation: u64,
    departed_groups: DepartedGroups,
    metadata_cache: MetadataCache,
    retry_tick: tokio::time::Interval,
    acknowledgment_retry_tick: tokio::time::Interval,
    group_sync_retry_tick: tokio::time::Interval,
    session: SessionState,
}

impl ConnectionSession {
    async fn handle_command_interruptibly(&mut self, command: Command) -> bool {
        let mut operation = Box::pin(handle_command(
            CommandContext {
                manager: &mut self.manager,
                repo: &self.repo,
                sink: &self.sink,
                departed_groups: &self.departed_groups,
                groups_authoritative: self.session.groups_authoritative(),
                metadata_cache: &self.metadata_cache,
                timestamps: &self.timestamps,
            },
            command,
        ));

        tokio::select! {
            () = &mut operation => false,
            _ = wait_for_shutdown(&mut self.shutdown) => true,
        }
    }

    async fn stop_and_drain(&mut self) {
        stop_attachments_and_drain_acknowledgments(
            &self.manager,
            &self.sink,
            &mut self.attachment_tasks,
            &mut self.attachment_aborts,
            &self.acknowledgments,
            &mut self.projection,
        )
        .await;
    }

    async fn stop_receive_and_drain(&mut self, receive_task: &mut tokio::task::JoinHandle<()>) {
        stop_receive_driver(receive_task).await;
        self.stop_and_drain().await;
    }

    async fn stop_active_and_drain(&mut self, receive_tasks: &mut ActiveReceiveTasks) {
        stop_active_receive_loop(
            receive_tasks,
            &self.manager,
            &self.sink,
            &mut self.attachment_tasks,
            &mut self.attachment_aborts,
            &self.acknowledgments,
            &mut self.projection,
        )
        .await;
    }

    /// Runs while `session.is_recovering()`: drains deferred commands, then
    /// waits out the backoff delay while still servicing acks and attachment
    /// completions. `Break` means the worker should exit outright (shutdown,
    /// or the command channel closed); `Continue` means recovery is no longer
    /// in effect and the caller should (re)connect. There is no exhaustion
    /// path here: `next_recovery_delay` keeps retrying indefinitely for as
    /// long as the underlying error stays transient (see its doc comment) -
    /// a permanent give-up only happens earlier, for errors already
    /// classified non-transient.
    async fn wait_out_recovery(&mut self) -> ControlFlow<Result<(), String>> {
        if !self.session.is_recovering() {
            return ControlFlow::Continue(());
        }
        if drain_recovery_commands(&mut self.commands, &mut self.deferred_commands) {
            self.stop_and_drain().await;
            return ControlFlow::Break(Ok(()));
        }
        let delay = self.session.next_recovery_delay();
        if delay.is_zero() {
            return ControlFlow::Continue(());
        }
        let sleep = tokio::time::sleep(delay);
        pin_mut!(sleep);
        loop {
            tokio::select! {
                _ = &mut sleep => break,
                command = self.commands.recv() => {
                    let Some(command) = command else {
                        self.stop_and_drain().await;
                        return ControlFlow::Break(Ok(()));
                    };
                    handle_recovery_command(command, &mut self.deferred_commands);
                }
                _ = self.acknowledgments.wait() => {
                    let phase = process_acknowledgments(
                        &self.manager,
                        &self.acknowledgments,
                        &self.sink,
                        &mut self.projection,
                        true,
                    );
                    if await_or_shutdown(phase, &mut self.shutdown).await.is_none() {
                        self.stop_and_drain().await;
                        return ControlFlow::Break(Ok(()));
                    }
                }
                _ = self.acknowledgment_retry_tick.tick() => {
                    self.acknowledgments.activate_retries();
                }
                completed = self.attachment_tasks.join_next(),
                    if !self.attachment_tasks.is_empty() =>
                {
                    if let Some(completed) = completed {
                        let phase = handle_attachment_completion(
                            &self.manager,
                            &self.sink,
                            &mut self.attachment_aborts,
                            completed,
                        );
                        if await_or_shutdown(phase, &mut self.shutdown).await.is_none() {
                            self.stop_and_drain().await;
                            return ControlFlow::Break(Ok(()));
                        }
                    }
                }
                _ = wait_for_shutdown(&mut self.shutdown) => {
                    self.stop_and_drain().await;
                    return ControlFlow::Break(Ok(()));
                },
            }
        }
        ControlFlow::Continue(())
    }

    /// Awaits the receive driver's startup signal, servicing acks, attachment
    /// completions, and (while still recovering) deferred commands in the
    /// meantime. `Break` means the worker should exit; `Continue` carries the
    /// resolved startup result.
    async fn wait_for_receive_start(
        &mut self,
        receive_started: &mut Pin<Box<oneshot::Receiver<Result<(), ReceiveStartError>>>>,
        receive_task: &mut tokio::task::JoinHandle<()>,
    ) -> ControlFlow<Result<(), String>, Result<(), ReceiveStartError>> {
        loop {
            tokio::select! {
                result = &mut *receive_started => {
                    return ControlFlow::Continue(result.unwrap_or_else(|_| {
                        Err(ReceiveStartError {
                            message: "Signal message reception stopped during startup".to_owned(),
                            transient: true,
                        })
                    }));
                }
                command = self.commands.recv(), if self.session.is_recovering() => {
                    let Some(command) = command else {
                        self.stop_receive_and_drain(receive_task).await;
                        return ControlFlow::Break(Ok(()));
                    };
                    handle_recovery_command(command, &mut self.deferred_commands);
                }
                _ = self.acknowledgments.wait() => {
                    let phase = process_acknowledgments(
                        &self.manager,
                        &self.acknowledgments,
                        &self.sink,
                        &mut self.projection,
                        true,
                    );
                    if await_or_shutdown(phase, &mut self.shutdown).await.is_none() {
                        self.stop_receive_and_drain(receive_task).await;
                        return ControlFlow::Break(Ok(()));
                    }
                }
                _ = self.acknowledgment_retry_tick.tick() => {
                    self.acknowledgments.activate_retries();
                }
                completed = self.attachment_tasks.join_next(),
                    if self.session.is_recovering() && !self.attachment_tasks.is_empty() =>
                {
                    if let Some(completed) = completed {
                        let phase = handle_attachment_completion(
                            &self.manager,
                            &self.sink,
                            &mut self.attachment_aborts,
                            completed,
                        );
                        if await_or_shutdown(phase, &mut self.shutdown).await.is_none() {
                            self.stop_receive_and_drain(receive_task).await;
                            return ControlFlow::Break(Ok(()));
                        }
                    }
                }
                _ = wait_for_shutdown(&mut self.shutdown) => {
                    self.stop_receive_and_drain(receive_task).await;
                    return ControlFlow::Break(Ok(()));
                },
            }
        }
    }

    /// Runs the one-time startup sequence (account identity, contact/group
    /// snapshots, unprojected message replay, identity changes, outbox retry)
    /// and marks the session ready. A no-op once the session is already ready
    /// (a reconnect that never lost readiness skips straight past it).
    async fn run_startup_phases(&mut self) -> ControlFlow<Result<(), String>> {
        if self.session.is_ready() {
            return ControlFlow::Continue(());
        }
        macro_rules! phase_or_stop {
            ($phase:expr) => {
                match await_or_shutdown($phase, &mut self.shutdown).await {
                    Some(output) => output,
                    None => {
                        self.stop_and_drain().await;
                        return ControlFlow::Break(Ok(()));
                    }
                }
            };
        }
        phase_or_stop!(emit_account_identity(&mut self.manager, &self.sink));
        phase_or_stop!(emit_contact_snapshot(
            &self.manager,
            &self.sink,
            &self.avatar_cache,
            &self.metadata_cache
        ));
        if let Err(error) = phase_or_stop!(emit_group_snapshot(
            &self.manager,
            &self.sink,
            &self.departed_groups,
            &self.avatar_cache,
            &self.metadata_cache,
        )) {
            self.sink.emit(Event::transient_error(error));
        }
        phase_or_stop!(load_unprojected_messages(
            &self.manager,
            &self.sink,
            &mut self.replay,
            self.session.groups_authoritative(),
        ));
        phase_or_stop!(emit_identity_changes(&self.manager, &self.sink));
        phase_or_stop!(retry_outbox(
            &mut self.manager,
            &self.repo,
            &self.sink,
            &self.departed_groups,
            &self.metadata_cache,
            self.session.groups_authoritative(),
        ));
        self.session.mark_ready();
        self.projection.delivery_receipts.activate_retries();
        self.ready.store(true, Ordering::Release);
        self.sink.emit(Event {
            kind: EVENT_READY,
            ..Event::default()
        });
        ControlFlow::Continue(())
    }

    /// The steady-state dispatch loop for one connected "generation": drains
    /// ready replayed content, starts delivery-receipt attempts, and services
    /// incoming messages/commands/attachment and delivery-receipt completions,
    /// and outbox/group-sync retries. `Break` means the worker should exit;
    /// `Continue(error)` means the connection needs to recover.
    async fn run_active_dispatch(
        &mut self,
        receive_tasks: &mut ActiveReceiveTasks,
        messages: &mut tokio_mpsc::Receiver<Received>,
        contact_sync_start: &mut Option<oneshot::Sender<()>>,
        avatar_fetch_start: &mut Option<oneshot::Sender<()>>,
        group_sync_tx: &tokio_mpsc::Sender<Result<(), String>>,
        group_sync_rx: &mut tokio_mpsc::Receiver<Result<(), String>>,
    ) -> ControlFlow<Result<(), String>, String> {
        macro_rules! phase_or_stop {
            ($phase:expr) => {
                match await_or_shutdown($phase, &mut self.shutdown).await {
                    Some(output) => output,
                    None => {
                        self.stop_active_and_drain(receive_tasks).await;
                        return ControlFlow::Break(Ok(()));
                    }
                }
            };
        }
        loop {
            let session_ready = self.session.is_ready();
            let groups_authoritative = self.session.groups_authoritative();
            let groups_dirty = self.session.groups_dirty();
            if groups_authoritative {
                self.replay.activate_groups();
            }
            if self.projection.has_capacity()
                && let Some(content) = self.replay.pop_ready()
            {
                phase_or_stop!(project_content(
                    &mut self.manager,
                    &self.repo,
                    content,
                    &self.sink,
                    &mut self.projection,
                    &self.departed_groups,
                    groups_authoritative,
                    &self.timestamps,
                ));
                continue;
            }
            if self.projection.delivery_receipt_tasks.is_empty()
                && let Some(receipt) = self.projection.delivery_receipts.start_next(session_ready)
            {
                spawn_delivery_receipt_attempt(
                    &mut self.projection.delivery_receipt_tasks,
                    self.manager.clone(),
                    self.receive_generation,
                    receipt,
                );
            }
            tokio::select! {
                _ = self.acknowledgments.wait() => {
                    phase_or_stop!(process_acknowledgments(
                        &self.manager,
                        &self.acknowledgments,
                        &self.sink,
                        &mut self.projection,
                        true,
                    ));
                }
                received = messages.recv(), if self.projection.has_capacity()
                    && self.replay.can_accept_live_message() => {
                    match received {
                        Some(Received::QueueEmpty) => {
                            if let Some(start) = contact_sync_start.take() {
                                let _ = start.send(());
                            }
                            if let Some(start) = avatar_fetch_start.take() {
                                let _ = start.send(());
                            }
                            if groups_dirty {
                                match phase_or_stop!(emit_group_snapshot(
                                    &self.manager,
                                    &self.sink,
                                    &self.departed_groups,
                                    &self.avatar_cache,
                                    &self.metadata_cache,
                                )) {
                                    Ok(()) => self.session.mark_groups_authoritative(),
                                    Err(error) => {
                                        self.session.mark_groups_pending();
                                        self.group_sync_retry_tick.reset();
                                        self.sink.emit(Event::transient_error(error));
                                    }
                                }
                            }
                        }
                        Some(Received::Contacts) => {
                            phase_or_stop!(emit_contact_snapshot(
                                &self.manager,
                                &self.sink,
                                &self.avatar_cache,
                                &self.metadata_cache,
                            ));
                        }
                        Some(Received::Content(content)) => {
                            self.session.note_group_content(content_has_group_context(&content.body));
                            if session_ready {
                                self.replay.push(*content, groups_authoritative);
                                phase_or_stop!(emit_identity_changes(&self.manager, &self.sink));
                            }
                        }
                        None => return ControlFlow::Continue(
                            "Signal's message stream ended unexpectedly".to_owned(),
                        ),
                    }
                }
                command = async {
                    if session_ready
                        && let Some(command) = self.deferred_commands.pop_front()
                    {
                        Some(command)
                    } else {
                        self.commands.recv().await
                    }
                } => {
                    let Some(command) = command else {
                        self.stop_active_and_drain(receive_tasks).await;
                        return ControlFlow::Break(Ok(()));
                    };
                    if !session_ready {
                        handle_recovery_command(command, &mut self.deferred_commands);
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
                                    self.sink.emit(Event::request_error(
                                        request_id,
                                        "Signal groups are temporarily unavailable until authoritative synchronization succeeds",
                                    ));
                                }
                                continue;
                            }
                            let mut attachment_manager = self.manager.clone();
                            let attachment_repo = self.repo.clone();
                            let attachment_departed_groups = self.departed_groups.clone();
                            let attachment_metadata_cache = self.metadata_cache.clone();
                            let attachment_timestamps = self.timestamps.clone();
                            let control = permit.control();
                            let task = self.attachment_tasks.spawn_local(async move {
                                attachment_task_result(
                                    request_id,
                                    permit,
                                    upload_and_send_attachment(
                                        &mut attachment_manager,
                                        &attachment_repo,
                                        OutgoingAttachment {
                                            recipient,
                                            filename,
                                            content_type,
                                            data,
                                            group,
                                        },
                                        &attachment_departed_groups,
                                        &attachment_metadata_cache,
                                        &attachment_timestamps,
                                    ),
                                )
                                .await
                            });
                            self.attachment_aborts.insert(
                                request_id,
                                AttachmentTaskControl { task, control },
                            );
                        }
                        command => {
                            if groups_authoritative
                                && let Command::LeaveGroup { group_key, .. } = &command
                            {
                                self.departed_groups.begin_leave(group_key.clone());
                            }
                            if self.handle_command_interruptibly(command).await {
                                self.stop_active_and_drain(receive_tasks).await;
                                return ControlFlow::Break(Ok(()));
                            }
                            phase_or_stop!(emit_identity_changes(&self.manager, &self.sink));
                        }
                    }
                }
                completed = self.attachment_tasks.join_next(), if !self.attachment_tasks.is_empty() => {
                    if let Some(completed) = completed {
                        phase_or_stop!(handle_attachment_completion(
                            &self.manager,
                            &self.sink,
                            &mut self.attachment_aborts,
                            completed,
                        ));
                    }
                }
                completed = self.projection.delivery_receipt_tasks.join_next(),
                    if !self.projection.delivery_receipt_tasks.is_empty() =>
                {
                    // The `if !is_empty()` guard should make this branch unreachable when
                    // `join_next()` returns `None`, but a defensive `continue` costs nothing
                    // and is safer than panicking the whole worker on a guard/poll mismatch.
                    let Some(completed) = completed else {
                        continue;
                    };
                    match completed {
                        Ok(completion) => {
                            let generation = completion.generation;
                            if let Some(error) = finish_delivery_receipt_attempt(
                                &mut self.projection.delivery_receipts,
                                &self.sink,
                                &completion,
                            ) && generation == self.receive_generation {
                                return ControlFlow::Continue(error);
                            }
                        }
                        Err(error) => {
                            self.projection.delivery_receipts.release_in_flight();
                            self.sink.emit(Event::error(
                                format!(
                                    "Signal delivery receipt worker stopped unexpectedly: {error}"
                                ),
                                false,
                            ));
                        }
                    }
                }
                _ = self.retry_tick.tick(), if session_ready => {
                    self.projection.delivery_receipts.activate_retries();
                    phase_or_stop!(retry_outbox(
                        &mut self.manager,
                        &self.repo,
                        &self.sink,
                        &self.departed_groups,
                        &self.metadata_cache,
                        groups_authoritative,
                    ));
                }
                _ = self.acknowledgment_retry_tick.tick() => {
                    self.acknowledgments.activate_retries();
                }
                result = group_sync_rx.recv() => {
                    let Some(result) = result else {
                        continue;
                    };
                    receive_tasks.group_sync = None;
                    match result {
                        Ok(()) => {
                            self.session.mark_groups_authoritative();
                            self.replay.activate_groups();
                            phase_or_stop!(retry_outbox(
                                &mut self.manager,
                                &self.repo,
                                &self.sink,
                                &self.departed_groups,
                                &self.metadata_cache,
                                true,
                            ));
                        }
                        Err(error) => {
                            self.session.mark_groups_pending();
                            self.sink.emit(Event::transient_error(error));
                            self.group_sync_retry_tick.reset();
                        }
                    }
                }
                _ = self.group_sync_retry_tick.tick(), if session_ready && !groups_authoritative && receive_tasks.group_sync.is_none() => {
                    receive_tasks.group_sync = Some(spawn_group_sync(
                        self.manager.clone(),
                        self.sink.clone(),
                        self.departed_groups.clone(),
                        self.avatar_cache.clone(),
                        self.metadata_cache.clone(),
                        self.shutdown.clone(),
                        group_sync_tx.clone(),
                    ));
                }
                _ = wait_for_shutdown(&mut self.shutdown) => {
                    self.stop_active_and_drain(receive_tasks).await;
                    return ControlFlow::Break(Ok(()));
                },
            }
        }
    }

    async fn run(mut self) -> Result<(), String> {
        loop {
            if let ControlFlow::Break(result) = self.wait_out_recovery().await {
                return result;
            }

            let (receive_started, mut messages, mut receive_task) =
                spawn_receive_driver(self.manager.clone());
            let mut receive_started = Box::pin(receive_started);

            let receive_started = match self
                .wait_for_receive_start(&mut receive_started, &mut receive_task)
                .await
            {
                ControlFlow::Break(result) => return result,
                ControlFlow::Continue(output) => output,
            };
            if let Err(ReceiveStartError { message, transient }) = receive_started {
                stop_receive_driver(&mut receive_task).await;
                let error = message;
                self.ready.store(false, Ordering::Release);
                if !transient {
                    fail_deferred_commands(
                        &self.sink,
                        &mut self.deferred_commands,
                        "Signal connection recovery stopped before the request could be sent",
                    );
                    self.stop_and_drain().await;
                    return Err(error);
                }
                let transition = self.session.enter_recovery(error.clone());
                if transition == RecoveryTransition::Entered {
                    self.sink.emit(Event {
                        kind: EVENT_RECOVERING,
                        ..Event::default()
                    });
                }
                self.sink.emit(Event::transient_error(format!(
                    "{error}; retrying automatically"
                )));
                continue;
            }
            self.receive_generation = self.receive_generation.wrapping_add(1).max(1);

            let (contact_sync_start, contact_sync_wait) = oneshot::channel();
            let mut contact_sync_start = Some(contact_sync_start);
            let contact_sync = tokio::task::spawn_local(request_contacts_after_queue_drain(
                contact_sync_wait,
                self.manager.clone(),
                self.shutdown.clone(),
                self.sink.clone(),
            ));
            let (avatar_fetch_start, avatar_fetch_wait) = oneshot::channel();
            let mut avatar_fetch_start = Some(avatar_fetch_start);
            let avatar_fetch = tokio::task::spawn_local(fetch_missing_avatars_after_queue_drain(
                avatar_fetch_wait,
                self.manager.clone(),
                self.shutdown.clone(),
                self.sink.clone(),
                self.avatar_cache.clone(),
                self.metadata_cache.clone(),
            ));
            let (group_sync_tx, mut group_sync_rx) = tokio_mpsc::channel(1);
            let group_sync = spawn_group_sync(
                self.manager.clone(),
                self.sink.clone(),
                self.departed_groups.clone(),
                self.avatar_cache.clone(),
                self.metadata_cache.clone(),
                self.shutdown.clone(),
                group_sync_tx.clone(),
            );
            let mut receive_tasks = ActiveReceiveTasks {
                receive: receive_task,
                contact_sync,
                avatar_fetch,
                group_sync: Some(group_sync),
            };

            if let ControlFlow::Break(result) = self.run_startup_phases().await {
                return result;
            }

            let recovery_error = match self
                .run_active_dispatch(
                    &mut receive_tasks,
                    &mut messages,
                    &mut contact_sync_start,
                    &mut avatar_fetch_start,
                    &group_sync_tx,
                    &mut group_sync_rx,
                )
                .await
            {
                ControlFlow::Break(result) => return result,
                ControlFlow::Continue(error) => error,
            };

            self.ready.store(false, Ordering::Release);
            let error = recovery_error;
            let transition = self.session.enter_recovery(error.clone());
            if transition == RecoveryTransition::Entered {
                self.sink.emit(Event {
                    kind: EVENT_RECOVERING,
                    ..Event::default()
                });
            }
            stop_receive_tasks(
                &mut receive_tasks,
                &self.manager,
                &self.sink,
                &mut self.attachment_tasks,
                &mut self.attachment_aborts,
                std::future::ready(()),
            )
            .await;
            self.sink.emit(Event::transient_error(format!(
                "{error}; reconnecting automatically"
            )));
        }
    }
}

async fn receive_and_command_loop(
    manager: Manager<SqliteStore, Registered>,
    commands: tokio_mpsc::Receiver<Command>,
    acknowledgments: Arc<AcknowledgmentInbox>,
    mut shutdown: watch::Receiver<bool>,
    sink: EventSink,
    ready: Arc<AtomicBool>,
    avatar_cache: AvatarCache,
) -> Result<(), String> {
    let repo = StorageRepository::new(manager.store().clone());
    let Some(init_result) = await_or_shutdown(repo.initialize_subsystems(), &mut shutdown).await
    else {
        return Ok(());
    };
    init_result.map_err(|error| error.to_string())?;

    let mut retry_tick = tokio::time::interval(std::time::Duration::from_secs(5));
    retry_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut acknowledgment_retry_tick = tokio::time::interval(std::time::Duration::from_secs(5));
    acknowledgment_retry_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    acknowledgment_retry_tick.reset();
    let mut group_sync_retry_tick =
        tokio::time::interval(std::time::Duration::from_secs(GROUP_SYNC_RETRY_SECS));
    group_sync_retry_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    group_sync_retry_tick.reset();

    let projection = MessageProjection::new(Arc::clone(&acknowledgments));

    let connection = ConnectionSession {
        manager,
        commands,
        acknowledgments,
        shutdown,
        sink,
        ready,
        avatar_cache,
        repo,
        timestamps: MessageTimestampAllocator::default(),
        projection,
        replay: MessageReplayQueue::default(),
        deferred_commands: VecDeque::new(),
        attachment_tasks: tokio::task::JoinSet::new(),
        attachment_aborts: HashMap::new(),
        receive_generation: 0,
        departed_groups: DepartedGroups::default(),
        metadata_cache: MetadataCache::default(),
        retry_tick,
        acknowledgment_retry_tick,
        group_sync_retry_tick,
        session: SessionState::default(),
    };

    connection.run().await
}

pub(crate) fn spawn_receive_driver(
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

pub(crate) async fn forward_stream_to_channel<S, T>(stream: S, sender: tokio_mpsc::Sender<T>)
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

pub(crate) fn handle_recovery_command(command: Command, deferred_commands: &mut VecDeque<Command>) {
    match command {
        Command::SetTyping { .. } => {}
        command => {
            deferred_commands.push_back(command);
        }
    }
}

pub(crate) fn interrupted_attachment_event(
    request_id: u64,
    control: &crate::attachment::AttachmentControl,
) -> Option<Event> {
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

pub(crate) fn interrupt_remaining_attachments(
    sink: &EventSink,
    attachment_controls: &mut HashMap<u64, AttachmentTaskControl>,
) {
    for (request_id, control) in attachment_controls.drain() {
        if let Some(event) = interrupted_attachment_event(request_id, &control.control) {
            sink.emit(event);
        }
    }
}

pub(crate) fn abandon_timed_out_attachments(
    sink: &EventSink,
    attachment_tasks: &mut tokio::task::JoinSet<AttachmentCompletion>,
    attachment_controls: &mut HashMap<u64, AttachmentTaskControl>,
) {
    let mut abandoned_tasks = std::mem::replace(attachment_tasks, tokio::task::JoinSet::new());
    abandoned_tasks.abort_all();
    drop(abandoned_tasks);
    interrupt_remaining_attachments(sink, attachment_controls);
}

pub(crate) async fn abort_and_drain_tasks<T: Send + 'static>(
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

pub(crate) async fn abort_in_flight_attachments(
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
        if let Some(sent) =
            super::coordinator::finish_attachment_completion(sink, attachment_aborts, completed)
        {
            sent_messages.push(sent);
        }
    }
    if !sent_messages.is_empty() {
        let repo = StorageRepository::new(manager.store().clone());
        for sent in sent_messages {
            super::coordinator::mark_sent_message_projected_or_report(&repo, &sent, sink).await;
        }
    }
    interrupt_remaining_attachments(sink, attachment_aborts);
}

pub(crate) async fn stop_attachments_and_drain_acknowledgments(
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

pub(crate) async fn stop_receive_tasks<F>(
    tasks: &mut ActiveReceiveTasks,
    manager: &Manager<SqliteStore, Registered>,
    sink: &EventSink,
    attachment_tasks: &mut tokio::task::JoinSet<super::coordinator::AttachmentCompletion>,
    attachment_controls: &mut HashMap<u64, AttachmentTaskControl>,
    final_cleanup: F,
) where
    F: Future<Output = ()>,
{
    tasks.receive.abort();
    tasks.contact_sync.abort();
    tasks.avatar_fetch.abort();
    if let Some(group_sync) = tasks.group_sync.as_mut() {
        group_sync.abort();
    }
    let cleanup = async {
        let _ = (&mut tasks.receive).await;
        let _ = (&mut tasks.contact_sync).await;
        let _ = (&mut tasks.avatar_fetch).await;
        if let Some(group_sync) = tasks.group_sync.as_mut() {
            let _ = group_sync.await;
        }
        abort_in_flight_attachments(manager, sink, attachment_tasks, attachment_controls).await;
        final_cleanup.await;
    };
    if !finish_shutdown_cleanup(cleanup, SHUTDOWN_CLEANUP_TIMEOUT).await {
        abandon_timed_out_attachments(sink, attachment_tasks, attachment_controls);
    }
}

pub(crate) async fn stop_active_receive_loop(
    tasks: &mut ActiveReceiveTasks,
    manager: &Manager<SqliteStore, Registered>,
    sink: &EventSink,
    attachment_tasks: &mut tokio::task::JoinSet<super::coordinator::AttachmentCompletion>,
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

pub(crate) fn drain_recovery_commands(
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

pub(crate) fn deferred_command_failure(command: Command, message: &str) -> Option<Event> {
    match command {
        Command::LeaveGroup {
            request_id,
            group_key,
        } => Some(Event::group_request_error(request_id, group_key, message)),
        Command::SendMessage { request_id, .. }
        | Command::SendGroupMessage { request_id, .. }
        | Command::AcceptIdentity { request_id, .. }
        | Command::DismissIdentity { request_id, .. }
        | Command::ResetSession { request_id, .. } => {
            Some(Event::request_error(request_id, message))
        }
        Command::SendAttachment {
            request_id, permit, ..
        } if permit.claim_terminal() => Some(Event::request_error(request_id, message)),
        Command::SendAttachment { .. } => None,
        Command::SetTyping { .. } => None,
        Command::MarkRead { .. } => None,
    }
}

pub(crate) fn fail_deferred_commands(
    sink: &EventSink,
    commands: &mut VecDeque<Command>,
    message: &str,
) {
    while let Some(command) = commands.pop_front() {
        if let Some(event) = deferred_command_failure(command, message) {
            sink.emit(event);
        }
    }
}

pub(crate) async fn attachment_task_result(
    request_id: u64,
    mut permit: AttachmentPermit,
    task: impl Future<Output = Result<SentMessage, String>>,
) -> AttachmentCompletion {
    let cancellation = permit.take_cancellation_registration();
    let task = std::panic::AssertUnwindSafe(task).catch_unwind();
    let result = match futures::future::Abortable::new(task, cancellation).await {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attachment::AttachmentAdmission;
    use crate::backend::coordinator::finish_attachment_completion;
    use crate::event::EVENT_ATTACHMENT_SENT;
    use std::path::PathBuf;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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
        let result = run_local_future(runtime, local, Box::pin(future), Duration::from_millis(10));
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
                data: AttachmentPayload::Data(b"attachment".to_vec()),
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
                        thread: presage::store::Thread::Group([0; 32]),
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
                        thread: presage::store::Thread::Group([0; 32]),
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
                    data: AttachmentPayload::Data(vec![1]),
                    group: false,
                    permit,
                },
                "recovery stopped",
            )
            .is_none()
        );
    }

    #[test]
    fn fails_deferred_requests_but_drops_ephemeral_typing_and_read_receipts() {
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
        let read = deferred_command_failure(
            Command::MarkRead {
                request_id: 45,
                recipient: "recipient".into(),
                timestamp: 12345,
            },
            "recovery stopped",
        );

        let reset_session = deferred_command_failure(
            Command::ResetSession {
                request_id: 44,
                recipient: "recipient".into(),
            },
            "recovery stopped",
        )
        .unwrap();

        assert_eq!(send.request_id, 41);
        assert_eq!(send.text.as_deref(), Some("recovery stopped"));
        assert_eq!(leave.request_id, 42);
        assert_eq!(leave.chat_id.as_deref(), Some("group"));
        assert!(typing.is_none());
        assert!(read.is_none());
        assert_eq!(reset_session.request_id, 44);
        assert_eq!(reset_session.text.as_deref(), Some("recovery stopped"));
    }
}
