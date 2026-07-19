// SPDX-License-Identifier: AGPL-3.0-only
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{SystemTime, UNIX_EPOCH};

use futures::{StreamExt, channel::oneshot, pin_mut};
use presage::libsignal_service::configuration::SignalServers;
use presage::libsignal_service::content::{Content, ContentBody, DataMessage, GroupContextV2};
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
use tokio::sync::{mpsc as tokio_mpsc, watch};
use zeroize::Zeroizing;

use crate::event::{
    EVENT_CONTACT, EVENT_CONTACT_SYNC_BEGIN, EVENT_CONTACT_SYNC_END, EVENT_DISCONNECTED,
    EVENT_GROUP, EVENT_GROUP_MESSAGE, EVENT_LINK_QR, EVENT_MESSAGE, EVENT_READY, EVENT_RECEIPT,
    EVENT_TYPING, Event, FLAG_OUTGOING,
};

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
        OnNewIdentity::Reject,
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
    let messages = {
        let receive = manager.receive_messages();
        pin_mut!(receive);
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

    loop {
        tokio::select! {
            received = messages.next() => {
                match received {
                    Some(Received::QueueEmpty) => {
                        emit_store_snapshot(&manager, &sink).await;
                        if !synchronized {
                            synchronized = true;
                            ready.store(true, Ordering::Release);
                            sink.emit(Event { kind: EVENT_READY, ..Event::default() });
                        }
                    }
                    Some(Received::Contacts) => emit_store_snapshot(&manager, &sink).await,
                    Some(Received::Content(content)) => {
                        handle_content(&mut manager, *content, &sink).await;
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
                        ).await {
                            return Ok(());
                        }
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
) -> bool {
    let operation = handle_command(manager, command, sink);
    pin_mut!(operation);

    tokio::select! {
        () = &mut operation => false,
        _ = wait_for_shutdown(shutdown) => true,
    }
}

async fn emit_store_snapshot(manager: &Manager<SqliteStore, Registered>, sink: &EventSink) {
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

    match manager.store().groups().await {
        Ok(groups) => match groups.collect::<Result<Vec<_>, _>>() {
            Ok(groups) => {
                for (key, group) in groups {
                    sink.emit(Event {
                        kind: EVENT_GROUP,
                        chat_id: Some(hex::encode(key)),
                        title: Some(group.title),
                        ..Event::default()
                    });
                }
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

async fn handle_command(
    manager: &mut Manager<SqliteStore, Registered>,
    command: Command,
    sink: &EventSink,
) {
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
            let result = match decode_group_key(&group_key) {
                Some(key) => {
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
                None => Err("Group master key is invalid".into()),
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
    };

    if let Err(error) = result {
        sink.emit(Event::request_error(request_id, error));
    }
}

async fn handle_content(
    manager: &mut Manager<SqliteStore, Registered>,
    content: Content,
    sink: &EventSink,
) {
    let timestamp = content_timestamp(&content);
    let sender = content.metadata.sender.service_id_string();

    match &content.body {
        ContentBody::DataMessage(message) => {
            emit_data_message(manager, message, &sender, false, timestamp, sink).await;
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
        }
        ContentBody::EditMessage(EditMessage {
            data_message: Some(message),
            ..
        }) => {
            emit_data_message(manager, message, &sender, false, timestamp, sink).await;
        }
        ContentBody::SynchronizeMessage(SyncMessage {
            sent: Some(sent), ..
        }) => {
            if let Some(message) = sent.message.as_ref() {
                let peer = sent
                    .parse_destination_service_id()
                    .map_or_else(|| sender.clone(), |id| id.service_id_string());
                emit_data_message(manager, message, &peer, true, timestamp, sink).await;
            } else if let Some(EditMessage {
                data_message: Some(message),
                ..
            }) = sent.edit_message.as_ref()
            {
                let peer = sent
                    .parse_destination_service_id()
                    .map_or_else(|| sender.clone(), |id| id.service_id_string());
                emit_data_message(manager, message, &peer, true, timestamp, sink).await;
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
}

async fn emit_data_message(
    manager: &Manager<SqliteStore, Registered>,
    message: &DataMessage,
    peer: &str,
    outgoing: bool,
    timestamp: u64,
    sink: &EventSink,
) {
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
        return;
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
            flags,
            peer_id: Some(group_peer),
            chat_id: Some(hex::encode(group_key)),
            title,
            text: Some(text),
            timestamp_ms: timestamp,
            ..Event::default()
        });
    } else {
        sink.emit(Event {
            kind: EVENT_MESSAGE,
            flags,
            peer_id: Some(peer.to_owned()),
            text: Some(text),
            timestamp_ms: timestamp,
            ..Event::default()
        });
    }
}

async fn send_delivery_receipt(
    manager: &mut Manager<SqliteStore, Registered>,
    recipient: ServiceId,
    message_timestamp: u64,
    sink: &EventSink,
) {
    let timestamp = now_ms();
    if let Err(error) = manager
        .send_message(
            recipient,
            ReceiptMessage {
                r#type: Some(receipt_message::Type::Delivery.into()),
                timestamp: vec![message_timestamp],
            },
            timestamp,
        )
        .await
    {
        sink.emit(Event::error(
            format!("Could not send a Signal delivery receipt: {error}"),
            false,
        ));
    }
}

fn parse_recipient(value: &str) -> Option<ServiceId> {
    ServiceId::parse_from_service_id_string(value).or_else(|| {
        value
            .parse::<presage::libsignal_service::prelude::Uuid>()
            .ok()
            .map(|uuid| ServiceId::Aci(uuid.into()))
    })
}

fn decode_group_key(value: &str) -> Option<[u8; 32]> {
    let bytes = hex::decode(value).ok()?;
    bytes.try_into().ok()
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
    fn validates_group_keys() {
        assert_eq!(decode_group_key(&"00".repeat(32)), Some([0; 32]));
        assert_eq!(decode_group_key("00"), None);
        assert_eq!(decode_group_key(&"zz".repeat(32)), None);
    }

    #[test]
    fn creates_a_png_qr_code() {
        let png = qr_png(b"sgnl://linkdevice?uuid=test&pub_key=test").unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(png.len() > 100);
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
