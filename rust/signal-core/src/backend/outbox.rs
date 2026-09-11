// SPDX-License-Identifier: AGPL-3.0-only
use presage::libsignal_service::content::{DataMessage, GroupContextV2};
use presage::store::Thread;
use presage_store_sqlite::{ClientOutboxKind, ClientOutboxMessage};

use super::coordinator::{DepartedGroups, MessageTimestampAllocator, SentMessage, wall_clock_ms};
use super::protocol::SignalProtocol;
use crate::event::Event;
use crate::event_queue::EventSink;
use crate::store::errors::sqlite_store_error_is_transient;
use crate::store::traits::StorageOps;

#[derive(Debug)]
pub(crate) struct OutboxAttemptError {
    message: String,
    retryable: bool,
}

impl OutboxAttemptError {
    pub(crate) fn permanent(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: false,
        }
    }

    pub(crate) fn retryable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: true,
        }
    }

    pub(crate) fn should_retry(&self) -> bool {
        self.retryable
    }
}

impl std::fmt::Display for OutboxAttemptError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

pub(crate) fn retry_delay_ms(attempts: u32) -> u64 {
    let exponent = attempts.min(9);
    5_000u64.saturating_mul(1u64 << exponent).min(3_600_000)
}

pub(crate) fn outbox_message_is_attemptable(
    kind: &ClientOutboxKind,
    groups_authoritative: bool,
) -> bool {
    groups_authoritative || matches!(kind, ClientOutboxKind::Direct)
}

pub(crate) async fn attempt_outbox_message<M: SignalProtocol>(
    manager: &mut M,
    message: &ClientOutboxMessage,
    departed_groups: &DepartedGroups,
    metadata_cache: &super::coordinator::MetadataCache,
) -> Result<SentMessage, OutboxAttemptError> {
    match message.kind {
        ClientOutboxKind::Direct => {
            let recipient =
                super::coordinator::parse_recipient(&message.recipient).ok_or_else(|| {
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
                    }
                    .into(),
                    message.timestamp,
                )
                .await
                .map_err(OutboxAttemptError::retryable)?;
            Ok(SentMessage {
                thread: Thread::Contact(recipient),
                timestamp: message.timestamp,
            })
        }
        ClientOutboxKind::Group => {
            // Held across resolve+send so a concurrent LeaveGroup can't depart the
            // group in the window between checking membership and actually sending,
            // matching the same guard already taken around attachment group sends
            // and around leaving a group.
            let _operation = departed_groups.lock_operation().await;
            let (key, group) = super::coordinator::resolve_active_group(
                manager,
                &message.recipient,
                departed_groups,
                metadata_cache,
            )
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
                    }
                    .into(),
                    message.timestamp,
                )
                .await
                .map_err(OutboxAttemptError::retryable)?;
            Ok(SentMessage {
                thread: Thread::Group(key),
                timestamp: message.timestamp,
            })
        }
    }
}

pub(crate) async fn finish_outbox_attempt(
    repo: &impl StorageOps,
    message: &ClientOutboxMessage,
    result: &Result<SentMessage, OutboxAttemptError>,
) -> Result<(), String> {
    match result {
        Ok(_) => repo
            .complete_outbox_message(message.id)
            .await
            .map_err(|error| {
                format!("Message sent but its outbox entry could not be cleared: {error}")
            }),
        Err(error) if !error.should_retry() => repo
            .complete_outbox_message(message.id)
            .await
            .map_err(|store_error| {
                format!("Could not discard a terminal outbox entry: {store_error}")
            }),
        Err(_) => {
            let attempts = message.attempts.saturating_add(1);
            repo.defer_outbox_message(
                message.id,
                attempts,
                wall_clock_ms().saturating_add(retry_delay_ms(attempts)),
            )
            .await
            .map_err(|error| format!("Could not schedule message retry: {error}"))
        }
    }
}

pub(crate) async fn retry_outbox<M: SignalProtocol>(
    manager: &mut M,
    repo: &impl StorageOps,
    sink: &EventSink,
    departed_groups: &DepartedGroups,
    metadata_cache: &super::coordinator::MetadataCache,
    groups_authoritative: bool,
) {
    let messages = match repo.due_outbox_messages(wall_clock_ms()).await {
        Ok(messages) => messages,
        Err(error) => {
            if sqlite_store_error_is_transient(&error) {
                tracing::warn!(%error, "Transient store contention reading encrypted Signal outbox; deferring");
            } else {
                sink.emit(Event::error(
                    format!("Could not read the encrypted Signal outbox: {error}"),
                    false,
                ));
            }
            return;
        }
    };
    for message in messages {
        if !outbox_message_is_attemptable(&message.kind, groups_authoritative) {
            continue;
        }
        let result =
            attempt_outbox_message(manager, &message, departed_groups, metadata_cache).await;
        if let Ok(sent) = &result {
            super::coordinator::mark_sent_message_projected_or_report(repo, sent, sink).await;
        }
        if let Err(error) = finish_outbox_attempt(repo, &message, &result).await {
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

pub(crate) struct NewOutboxMessage {
    pub(crate) kind: ClientOutboxKind,
    pub(crate) recipient: String,
    pub(crate) body: String,
}

pub(crate) async fn enqueue_and_send<M: SignalProtocol>(
    manager: &mut M,
    repo: &impl StorageOps,
    request: NewOutboxMessage,
    departed_groups: &DepartedGroups,
    metadata_cache: &super::coordinator::MetadataCache,
    sink: &EventSink,
    timestamps: &MessageTimestampAllocator,
) -> Result<(), String> {
    let NewOutboxMessage {
        kind,
        recipient,
        body,
    } = request;
    let timestamp = timestamps.next();
    let id = repo
        .enqueue_outbox_message(kind, &recipient, &body, timestamp)
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
    let result = attempt_outbox_message(manager, &message, departed_groups, metadata_cache).await;
    if let Ok(sent) = &result {
        super::coordinator::mark_sent_message_projected_or_report(repo, sent, sink).await;
    }
    finish_outbox_attempt(repo, &message, &result).await?;
    result.map(|_| ()).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn bounds_outbox_retry_backoff() {
        assert_eq!(retry_delay_ms(0), 5_000);
        assert_eq!(retry_delay_ms(1), 10_000);
        assert_eq!(retry_delay_ms(4), 80_000);
        assert_eq!(retry_delay_ms(32), 2_560_000);
    }
}
