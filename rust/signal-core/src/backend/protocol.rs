// SPDX-License-Identifier: AGPL-3.0-only
use presage::Manager;
use presage::libsignal_service::content::ContentBody;
use presage::libsignal_service::protocol::{Aci, ServiceId};
use presage::libsignal_service::sender::AttachmentSpec;
use presage::manager::{LeaveGroupOutcome, Registered};
use presage::proto::AttachmentPointer;
use presage_store_sqlite::SqliteStore;

/// The subset of `presage::Manager` operations this crate's command/content
/// handling drives. Generic code depends on this trait instead of the concrete
/// `Manager<SqliteStore, Registered>` so it can be exercised in tests against a
/// fake implementation, without a live Signal connection.
///
/// Every method already converts its underlying `presage`/store error to a
/// `String`: every call site converted it immediately anyway, so doing it once
/// here removes a repeated `.map_err(|error| error.to_string())` at each call.
pub(crate) trait SignalProtocol: Clone {
    fn local_aci(&self) -> Aci;

    fn store(&self) -> &SqliteStore;

    async fn send_message(
        &mut self,
        recipient: ServiceId,
        message: ContentBody,
        timestamp: u64,
    ) -> Result<(), String>;

    async fn send_message_to_group(
        &mut self,
        group_key: &[u8; 32],
        message: ContentBody,
        timestamp: u64,
    ) -> Result<(), String>;

    async fn upload_attachment(
        &self,
        spec: AttachmentSpec,
        contents: Vec<u8>,
    ) -> Result<AttachmentPointer, String>;

    async fn get_attachment(&self, pointer: &AttachmentPointer) -> Result<Vec<u8>, String>;

    async fn leave_group(&mut self, master_key: &[u8; 32]) -> Result<LeaveGroupOutcome, String>;

    async fn clear_sessions(&self, recipient: &ServiceId) -> Result<(), String>;
}

impl SignalProtocol for Manager<SqliteStore, Registered> {
    fn local_aci(&self) -> Aci {
        self.registration_data().service_ids.aci()
    }

    fn store(&self) -> &SqliteStore {
        Manager::store(self)
    }

    async fn send_message(
        &mut self,
        recipient: ServiceId,
        message: ContentBody,
        timestamp: u64,
    ) -> Result<(), String> {
        Manager::send_message(self, recipient, message, timestamp)
            .await
            .map_err(|error| error.to_string())
    }

    async fn send_message_to_group(
        &mut self,
        group_key: &[u8; 32],
        message: ContentBody,
        timestamp: u64,
    ) -> Result<(), String> {
        Manager::send_message_to_group(self, group_key, message, timestamp)
            .await
            .map_err(|error| error.to_string())
    }

    async fn upload_attachment(
        &self,
        spec: AttachmentSpec,
        contents: Vec<u8>,
    ) -> Result<AttachmentPointer, String> {
        Manager::upload_attachment(self, spec, contents)
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())
    }

    async fn get_attachment(&self, pointer: &AttachmentPointer) -> Result<Vec<u8>, String> {
        Manager::get_attachment(self, pointer)
            .await
            .map_err(|error| error.to_string())
    }

    async fn leave_group(&mut self, master_key: &[u8; 32]) -> Result<LeaveGroupOutcome, String> {
        Manager::leave_group(self, master_key)
            .await
            .map_err(|error| error.to_string())
    }

    async fn clear_sessions(&self, recipient: &ServiceId) -> Result<(), String> {
        Manager::clear_sessions(self, recipient)
            .await
            .map_err(|error| error.to_string())
    }
}
