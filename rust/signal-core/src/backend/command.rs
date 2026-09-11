// SPDX-License-Identifier: AGPL-3.0-only
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
#[cfg(test)]
use std::sync::atomic::Ordering;

use tokio::sync::{mpsc as tokio_mpsc, watch};
use zeroize::{Zeroize, Zeroizing};

use crate::acknowledgment::AcknowledgmentInbox;
use crate::attachment::{AttachmentPayload, AttachmentPermit};
use crate::event_queue::EventSink;

pub struct Config {
    pub store_path: String,
    pub device_name: String,
    pub passphrase: StorePassphrase,
}

pub struct WorkerContext {
    pub config: Config,
    pub commands: tokio_mpsc::Receiver<Command>,
    pub acknowledgments: Arc<AcknowledgmentInbox>,
    pub shutdown: watch::Receiver<bool>,
    pub events: EventSink,
    pub ready: Arc<AtomicBool>,
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
        data: AttachmentPayload,
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
    ResetSession {
        request_id: u64,
        recipient: String,
    },
    MarkRead {
        request_id: u64,
        recipient: String,
        timestamp: u64,
    },
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
