// SPDX-License-Identifier: AGPL-3.0-only
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{SystemTime, UNIX_EPOCH};

use futures::{StreamExt, channel::oneshot, pin_mut};
use presage::libsignal_service::configuration::SignalServers;
use presage::libsignal_service::content::{Content, ContentBody, DataMessage, GroupContextV2};
use presage::libsignal_service::groups_v2::Role;
use presage::libsignal_service::protocol::ServiceId;
use presage::model::identity::OnNewIdentity;
use presage::model::messages::Received;
use presage::proto::{
    EditMessage, ReceiptMessage, SyncMessage, TypingMessage, receipt_message, typing_message,
};
use presage::store::{ContentsStore, StateStore};
use presage::{Manager, manager::Registered};
use presage_store_sqlite::SqliteStore;
use qrcode::QrCode;
use qrcode::types::Color;
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc as tokio_mpsc, watch};
use zeroize::Zeroizing;

use crate::event::{
    EVENT_CONTACT, EVENT_CONTACT_SYNC_BEGIN, EVENT_CONTACT_SYNC_END, EVENT_DISCONNECTED,
    EVENT_GROUP, EVENT_GROUP_MEMBER, EVENT_GROUP_MESSAGE, EVENT_GROUP_SYNC_BEGIN,
    EVENT_GROUP_SYNC_END, EVENT_IDENTITY_ACCEPTED, EVENT_IDENTITY_CHANGE, EVENT_LINK_QR,
    EVENT_MESSAGE, EVENT_READY, EVENT_RECEIPT, EVENT_TYPING, Event, FLAG_OUTGOING,
};

const MESSAGE_PROJECTION_CLIENT: &str = "signal-purple-v1";

pub struct Config {
    pub store_path: String,
    pub device_name: String,
    pub passphrase: Zeroizing<String>,
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
}

impl MessageProjection {
    fn track(&mut self, content: Content) -> u64 {
        self.next_delivery_id = self.next_delivery_id.wrapping_add(1).max(1);
        let delivery_id = self.next_delivery_id;
        self.pending.insert(delivery_id, content);
        delivery_id
    }
}

#[derive(Clone)]
struct EventSink {
    sender: mpsc::SyncSender<Event>,
    overflowed: Arc<AtomicBool>,
}

impl EventSink {
    fn emit(&self, event: Event) {
        if self.overflowed.load(Ordering::Acquire) {
            return;
        }
        if matches!(
            self.sender.try_send(event),
            Err(mpsc::TrySendError::Full(_))
        ) {
            self.overflowed.store(true, Ordering::Release);
        }
    }
}

pub fn run_worker(
    config: Config,
    commands: tokio_mpsc::Receiver<Command>,
    shutdown: watch::Receiver<bool>,
    events: mpsc::SyncSender<Event>,
    overflowed: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
) {
    let sink = EventSink {
        sender: events,
        overflowed,
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
    let messages = {
        let mut receive = Box::pin(manager.receive_messages());
        tokio::select! {
            result = &mut receive => {
                result.map_err(|error| {
                    format!("Could not start Signal message reception: {error}")
                })?
            }
            _ = wait_for_shutdown(&mut shutdown) => return Ok(()),
        }
    };
    pin_mut!(messages);

    if let Err(error) = manager.request_contacts().await {
        sink.emit(Event::error(
            format!("Could not request Signal contact synchronization: {error}"),
            false,
        ));
    }

    let mut synchronized = false;
    let mut groups_dirty = false;
    let mut projection = MessageProjection::default();

    loop {
        tokio::select! {
            received = messages.next() => {
                match received {
                    Some(Received::QueueEmpty) => {
                        if !synchronized {
                            match Box::pin(manager.synchronize_storage_groups()).await {
                                Ok(_) => {}
                                Err(error) => sink.emit(Event::error(
                                    format!("Could not synchronize Signal groups: {error}"),
                                    false,
                                )),
                            }
                            emit_contact_snapshot(&manager, &sink).await;
                            emit_group_snapshot(&manager, &sink).await;
                            replay_unprojected_messages(
                                &mut manager,
                                &sink,
                                &mut projection,
                            ).await;
                            emit_identity_changes(&manager, &sink).await;
                            groups_dirty = false;
                            synchronized = true;
                            ready.store(true, Ordering::Release);
                            sink.emit(Event { kind: EVENT_READY, ..Event::default() });
                        } else if groups_dirty {
                            emit_group_snapshot(&manager, &sink).await;
                            groups_dirty = false;
                        }
                    }
                    Some(Received::Contacts) => emit_contact_snapshot(&manager, &sink).await,
                    Some(Received::Content(content)) => {
                        groups_dirty |= content_has_group_context(&content.body);
                        if synchronized {
                            project_content(
                                &mut manager,
                                *content,
                                &sink,
                                &mut projection,
                            ).await;
                            emit_identity_changes(&manager, &sink).await;
                        }
                    }
                    None => {
                        sink.emit(Event {
                            kind: EVENT_DISCONNECTED,
                            text: Some("Signal's message stream ended".into()),
                            ..Event::default()
                        });
                        return Ok(());
                    }
                }
            }
            command = commands.recv() => {
                match command {
                    None => return Ok(()),
                    Some(command) => {
                        if handle_command_interruptibly(
                            &mut manager,
                            command,
                            &mut shutdown,
                            &sink,
                            &mut projection,
                        ).await {
                            return Ok(());
                        }
                        emit_identity_changes(&manager, &sink).await;
                    }
                }
            }
            _ = wait_for_shutdown(&mut shutdown) => return Ok(()),
        }
    }
}

async fn handle_command_interruptibly(
    manager: &mut Manager<SqliteStore, Registered>,
    command: Command,
    shutdown: &mut watch::Receiver<bool>,
    sink: &EventSink,
    projection: &mut MessageProjection,
) -> bool {
    let operation = handle_command(manager, command, sink, projection);
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

async fn emit_group_snapshot(manager: &Manager<SqliteStore, Registered>, sink: &EventSink) {
    match manager.store().groups().await {
        Ok(groups) => match groups.collect::<Result<Vec<_>, _>>() {
            Ok(groups) => {
                sink.emit(Event {
                    kind: EVENT_GROUP_SYNC_BEGIN,
                    ..Event::default()
                });
                for (key, group) in groups {
                    let chat_id = group_identifier(&key);
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
            }
            Err(error) => sink.emit(Event::error(
                format!("Could not decode synchronized Signal groups: {error}"),
                false,
            )),
        },
        Err(error) => sink.emit(Event::error(
            format!("Could not read synchronized Signal groups: {error}"),
            false,
        )),
    }
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

async fn replay_unprojected_messages(
    manager: &mut Manager<SqliteStore, Registered>,
    sink: &EventSink,
    projection: &mut MessageProjection,
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
        project_content(manager, content, sink, projection).await;
    }
}

async fn project_content(
    manager: &mut Manager<SqliteStore, Registered>,
    content: Content,
    sink: &EventSink,
    projection: &mut MessageProjection,
) {
    let delivery_id = projection.track(content.clone());
    if handle_content(manager, content.clone(), delivery_id, sink).await {
        return;
    }

    projection.pending.remove(&delivery_id);
    if let Err(error) = manager
        .store()
        .mark_message_projected(MESSAGE_PROJECTION_CLIENT, &content)
        .await
    {
        sink.emit(Event::error(
            format!("Could not record a handled Signal message: {error}"),
            false,
        ));
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

async fn handle_command(
    manager: &mut Manager<SqliteStore, Registered>,
    command: Command,
    sink: &EventSink,
    projection: &mut MessageProjection,
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
                projection.pending.remove(&delivery_id);
            }
            Err(error) => sink.emit(Event::error(
                format!("Could not acknowledge a displayed Signal message: {error}"),
                false,
            )),
        }
        return;
    }

    if let Command::AcceptIdentity {
        request_id,
        recipient,
    } = command
    {
        match manager.store().accept_identity_change(&recipient).await {
            Ok(true) => sink.emit(Event {
                kind: EVENT_IDENTITY_ACCEPTED,
                request_id,
                peer_id: Some(recipient),
                ..Event::default()
            }),
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

    let (request_id, result) = match command {
        Command::SendMessage {
            request_id,
            recipient,
            message,
        } => {
            let result = match parse_recipient(&recipient) {
                Some(recipient) => {
                    let timestamp = now_ms();
                    manager
                        .send_message(
                            recipient,
                            DataMessage {
                                body: Some(message),
                                timestamp: Some(timestamp),
                                ..Default::default()
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
        Command::SendGroupMessage {
            request_id,
            group_key,
            message,
        } => {
            let result = match resolve_group_key(manager, &group_key).await {
                Ok(Some(key)) => {
                    let timestamp = now_ms();
                    let revision = manager
                        .store()
                        .group(key)
                        .await
                        .ok()
                        .flatten()
                        .map_or(0, |group| group.revision);
                    manager
                        .send_message_to_group(
                            &key,
                            DataMessage {
                                body: Some(message),
                                timestamp: Some(timestamp),
                                group_v2: Some(GroupContextV2 {
                                    master_key: Some(key.to_vec()),
                                    revision: Some(revision),
                                    ..Default::default()
                                }),
                                ..Default::default()
                            },
                            timestamp,
                        )
                        .await
                        .map_err(|error| error.to_string())
                }
                Ok(None) => Err("Signal group is not available in the encrypted store".into()),
                Err(error) => Err(error),
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
) -> bool {
    let timestamp = content_timestamp(&content);
    let sender = content.metadata.sender.service_id_string();

    match &content.body {
        ContentBody::DataMessage(message) => {
            let projected = emit_data_message(
                manager,
                message,
                &sender,
                false,
                timestamp,
                delivery_id,
                sink,
            )
            .await;
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
            return projected;
        }
        ContentBody::EditMessage(EditMessage {
            data_message: Some(message),
            ..
        }) => {
            return emit_data_message(
                manager,
                message,
                &sender,
                false,
                timestamp,
                delivery_id,
                sink,
            )
            .await;
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
                    message,
                    &peer,
                    true,
                    timestamp,
                    delivery_id,
                    sink,
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
                    message,
                    &peer,
                    true,
                    timestamp,
                    delivery_id,
                    sink,
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
    false
}

async fn emit_data_message(
    manager: &Manager<SqliteStore, Registered>,
    message: &DataMessage,
    peer: &str,
    outgoing: bool,
    timestamp: u64,
    delivery_id: u64,
    sink: &EventSink,
) -> bool {
    let mut text = message.body.clone().unwrap_or_default();
    if let Some(reaction) = &message.reaction
        && let Some(emoji) = &reaction.emoji
    {
        text = format!("Reacted with {emoji}");
    }
    for attachment in &message.attachments {
        let name = attachment.file_name.as_deref().unwrap_or("attachment");
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&format!("[Attachment: {name}]"));
    }
    if text.is_empty() {
        return false;
    }

    let group_key = message
        .group_v2
        .as_ref()
        .and_then(|group| group.master_key.as_deref())
        .and_then(|key| <[u8; 32]>::try_from(key).ok());
    let flags = if outgoing { FLAG_OUTGOING } else { 0 };

    if let Some(group_key) = group_key {
        let title = manager
            .store()
            .group(group_key)
            .await
            .ok()
            .flatten()
            .map(|group| group.title);
        let group_peer = if outgoing {
            manager
                .registration_data()
                .service_ids
                .aci()
                .service_id_string()
        } else {
            peer.to_owned()
        };
        sink.emit(Event {
            kind: EVENT_GROUP_MESSAGE,
            request_id: delivery_id,
            flags,
            peer_id: Some(group_peer),
            chat_id: Some(group_identifier(&group_key)),
            title,
            text: Some(text),
            timestamp_ms: timestamp,
            ..Event::default()
        });
    } else {
        sink.emit(Event {
            kind: EVENT_MESSAGE,
            request_id: delivery_id,
            flags,
            peer_id: Some(peer.to_owned()),
            text: Some(text),
            timestamp_ms: timestamp,
            ..Event::default()
        });
    }
    true
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

async fn resolve_group_key(
    manager: &Manager<SqliteStore, Registered>,
    identifier: &str,
) -> Result<Option<[u8; 32]>, String> {
    let groups = manager
        .store()
        .groups()
        .await
        .map_err(|error| format!("Could not read Signal groups: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not decode Signal groups: {error}"))?;
    Ok(groups
        .into_iter()
        .map(|(key, _)| key)
        .find(|key| group_identifier(key) == identifier))
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
    }

    #[test]
    fn reports_event_queue_overflow_without_growing_the_queue() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let overflowed = Arc::new(AtomicBool::new(false));
        let sink = EventSink {
            sender,
            overflowed: Arc::clone(&overflowed),
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
        assert_eq!(receiver.try_recv().unwrap().kind, EVENT_MESSAGE);
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
    }
}
