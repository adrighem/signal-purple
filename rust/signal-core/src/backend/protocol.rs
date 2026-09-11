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

#[cfg(test)]
pub(crate) mod fake {
    use std::sync::{Arc, Mutex};

    use presage::libsignal_service::protocol::ServiceId;

    use super::*;

    type SentGroupMessage = ([u8; 32], ContentBody, u64);

    #[derive(Clone, Default)]
    pub(crate) struct FakeSignalProtocol {
        pub aci: Option<Aci>,
        pub sent_messages: Arc<Mutex<Vec<(ServiceId, ContentBody, u64)>>>,
        pub sent_group_messages: Arc<Mutex<Vec<SentGroupMessage>>>,
        pub left_groups: Arc<Mutex<Vec<[u8; 32]>>>,
        pub cleared_sessions: Arc<Mutex<Vec<ServiceId>>>,
    }

    impl SignalProtocol for FakeSignalProtocol {
        fn local_aci(&self) -> Aci {
            self.aci.unwrap_or_else(|| {
                match ServiceId::parse_from_service_id_string(
                    "00000000-0000-4000-8000-000000000001",
                ) {
                    Some(ServiceId::Aci(aci)) => aci,
                    _ => unreachable!(),
                }
            })
        }

        async fn send_message(
            &mut self,
            recipient: ServiceId,
            message: ContentBody,
            timestamp: u64,
        ) -> Result<(), String> {
            self.sent_messages
                .lock()
                .unwrap()
                .push((recipient, message, timestamp));
            Ok(())
        }

        async fn send_message_to_group(
            &mut self,
            group_key: &[u8; 32],
            message: ContentBody,
            timestamp: u64,
        ) -> Result<(), String> {
            self.sent_group_messages
                .lock()
                .unwrap()
                .push((*group_key, message, timestamp));
            Ok(())
        }

        async fn upload_attachment(
            &self,
            _spec: AttachmentSpec,
            _contents: Vec<u8>,
        ) -> Result<AttachmentPointer, String> {
            Ok(AttachmentPointer::default())
        }

        async fn get_attachment(&self, _pointer: &AttachmentPointer) -> Result<Vec<u8>, String> {
            Ok(Vec::new())
        }

        async fn leave_group(
            &mut self,
            master_key: &[u8; 32],
        ) -> Result<LeaveGroupOutcome, String> {
            self.left_groups.lock().unwrap().push(*master_key);
            Ok(LeaveGroupOutcome {
                peer_notification_sent: true,
                local_group_removed: true,
            })
        }

        async fn clear_sessions(&self, recipient: &ServiceId) -> Result<(), String> {
            self.cleared_sessions.lock().unwrap().push(*recipient);
            Ok(())
        }
    }

    #[tokio::test]
    async fn fake_signal_protocol_records_operations() {
        let mut protocol = FakeSignalProtocol::default();
        let service_id =
            ServiceId::parse_from_service_id_string("00000000-0000-4000-8000-000000000001")
                .unwrap();
        protocol.clear_sessions(&service_id).await.unwrap();
        assert_eq!(*protocol.cleared_sessions.lock().unwrap(), vec![service_id]);

        let group_key = [42u8; 32];
        let outcome = protocol.leave_group(&group_key).await.unwrap();
        assert!(outcome.peer_notification_sent);
        assert_eq!(*protocol.left_groups.lock().unwrap(), vec![group_key]);
    }
}
