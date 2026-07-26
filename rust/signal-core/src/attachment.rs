// SPDX-License-Identifier: AGPL-3.0-only
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use futures::future::{AbortHandle, AbortRegistration};

pub(crate) const MAX_ATTACHMENT_BYTES: usize = 25 * 1024 * 1024;
const MAX_ADMITTED_OUTGOING_ATTACHMENT_BYTES: usize = 50 * 1024 * 1024;
const MAX_ADMITTED_OUTGOING_ATTACHMENTS: usize = 2;

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum AttachmentAdmissionError {
    Invalid,
    Capacity,
}

#[derive(Debug)]
struct AdmissionState {
    bytes: usize,
    requests: HashMap<u64, AdmissionEntry>,
}

#[derive(Debug)]
struct AdmissionEntry {
    bytes: usize,
    control: AttachmentControl,
}

const ATTACHMENT_ACTIVE: u8 = 0;
const ATTACHMENT_CANCELLED: u8 = 1;
const ATTACHMENT_TERMINAL: u8 = 2;

#[derive(Debug)]
struct AttachmentControlState {
    lifecycle: AtomicU8,
    cancellation: AbortHandle,
}

#[derive(Clone, Debug)]
pub(crate) struct AttachmentControl {
    state: Arc<AttachmentControlState>,
}

impl AttachmentControl {
    fn new(cancellation: AbortHandle) -> Self {
        Self {
            state: Arc::new(AttachmentControlState {
                lifecycle: AtomicU8::new(ATTACHMENT_ACTIVE),
                cancellation,
            }),
        }
    }

    fn cancel(&self) -> bool {
        if self
            .state
            .lifecycle
            .compare_exchange(
                ATTACHMENT_ACTIVE,
                ATTACHMENT_CANCELLED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return false;
        }
        self.state.cancellation.abort();
        true
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.state.lifecycle.load(Ordering::Acquire) == ATTACHMENT_CANCELLED
    }

    pub(crate) fn claim_terminal(&self) -> bool {
        self.state
            .lifecycle
            .compare_exchange(
                ATTACHMENT_ACTIVE,
                ATTACHMENT_TERMINAL,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }
}

#[derive(Debug)]
pub(crate) struct AttachmentAdmission {
    maximum_bytes: usize,
    maximum_attachments: usize,
    state: Mutex<AdmissionState>,
}

impl AttachmentAdmission {
    pub(crate) fn new() -> Arc<Self> {
        Self::with_limits(
            MAX_ADMITTED_OUTGOING_ATTACHMENT_BYTES,
            MAX_ADMITTED_OUTGOING_ATTACHMENTS,
        )
    }

    fn with_limits(maximum_bytes: usize, maximum_attachments: usize) -> Arc<Self> {
        Arc::new(Self {
            maximum_bytes,
            maximum_attachments,
            state: Mutex::new(AdmissionState {
                bytes: 0,
                requests: HashMap::new(),
            }),
        })
    }

    pub(crate) fn try_reserve(
        self: &Arc<Self>,
        request_id: u64,
        bytes: usize,
    ) -> Result<AttachmentPermit, AttachmentAdmissionError> {
        if request_id == 0 || bytes == 0 || bytes > MAX_ATTACHMENT_BYTES {
            return Err(AttachmentAdmissionError::Invalid);
        }

        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.requests.contains_key(&request_id) {
            return Err(AttachmentAdmissionError::Invalid);
        }
        if state.requests.len() >= self.maximum_attachments
            || state
                .bytes
                .checked_add(bytes)
                .is_none_or(|total| total > self.maximum_bytes)
        {
            return Err(AttachmentAdmissionError::Capacity);
        }

        let (cancellation, registration) = AbortHandle::new_pair();
        let control = AttachmentControl::new(cancellation);
        state.bytes += bytes;
        state.requests.insert(
            request_id,
            AdmissionEntry {
                bytes,
                control: control.clone(),
            },
        );
        Ok(AttachmentPermit {
            admission: Arc::clone(self),
            request_id,
            control,
            registration: Some(registration),
        })
    }

    pub(crate) fn cancel(&self, request_id: u64) -> bool {
        let control = {
            let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state
                .requests
                .get(&request_id)
                .map(|entry| entry.control.clone())
        };
        if let Some(control) = control {
            let _ = control.cancel();
            true
        } else {
            false
        }
    }

    pub(crate) fn cancel_all(&self) {
        let controls = {
            let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state
                .requests
                .values()
                .map(|entry| entry.control.clone())
                .collect::<Vec<_>>()
        };
        for control in controls {
            let _ = control.cancel();
        }
    }

    fn release(&self, request_id: u64) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(entry) = state.requests.remove(&request_id) {
            state.bytes = state.bytes.saturating_sub(entry.bytes);
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(maximum_bytes: usize, maximum_attachments: usize) -> Arc<Self> {
        Self::with_limits(maximum_bytes, maximum_attachments)
    }

    #[cfg(test)]
    pub(crate) fn usage(&self) -> (usize, usize) {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        (state.bytes, state.requests.len())
    }
}

#[derive(Debug)]
pub(crate) struct AttachmentPermit {
    admission: Arc<AttachmentAdmission>,
    request_id: u64,
    control: AttachmentControl,
    registration: Option<AbortRegistration>,
}

impl AttachmentPermit {
    pub(crate) fn is_cancelled(&self) -> bool {
        self.control.is_cancelled()
    }

    pub(crate) fn control(&self) -> AttachmentControl {
        self.control.clone()
    }

    pub(crate) fn claim_terminal(&self) -> bool {
        self.control.claim_terminal()
    }

    pub(crate) fn take_cancellation_registration(&mut self) -> AbortRegistration {
        self.registration
            .take()
            .expect("attachment cancellation registration is taken only once")
    }
}

impl Drop for AttachmentPermit {
    fn drop(&mut self) {
        self.admission.release(self.request_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforces_count_and_aggregate_byte_limits() {
        let admission = AttachmentAdmission::for_test(10, 2);
        let first = admission.try_reserve(1, 4).unwrap();
        let second = admission.try_reserve(2, 6).unwrap();

        assert_eq!(admission.usage(), (10, 2));
        assert_eq!(
            admission.try_reserve(3, 1).unwrap_err(),
            AttachmentAdmissionError::Capacity
        );

        drop(second);
        assert_eq!(
            admission.try_reserve(3, 7).unwrap_err(),
            AttachmentAdmissionError::Capacity
        );
        drop(first);
    }

    #[test]
    fn rejects_invalid_and_duplicate_reservations() {
        let admission = AttachmentAdmission::for_test(MAX_ATTACHMENT_BYTES * 2, 2);
        assert_eq!(
            admission.try_reserve(0, 1).unwrap_err(),
            AttachmentAdmissionError::Invalid
        );
        assert_eq!(
            admission.try_reserve(1, 0).unwrap_err(),
            AttachmentAdmissionError::Invalid
        );
        let _permit = admission.try_reserve(1, 1).unwrap();
        assert_eq!(
            admission.try_reserve(1, 1).unwrap_err(),
            AttachmentAdmissionError::Invalid
        );
    }

    #[test]
    fn dropping_a_permit_releases_its_request_id_and_bytes() {
        let admission = AttachmentAdmission::for_test(5, 1);
        let permit = admission.try_reserve(7, 5).unwrap();
        assert_eq!(admission.usage(), (5, 1));

        drop(permit);

        assert_eq!(admission.usage(), (0, 0));
        let _replacement = admission.try_reserve(7, 5).unwrap();
    }

    #[test]
    fn cancellation_is_recorded_on_the_admitted_request() {
        let admission = AttachmentAdmission::for_test(5, 1);
        let permit = admission.try_reserve(7, 5).unwrap();

        assert!(admission.cancel(7));
        assert!(permit.is_cancelled());
        assert!(admission.cancel(7));
    }

    #[test]
    fn permit_and_admission_share_cancellation_state() {
        let admission = AttachmentAdmission::for_test(5, 1);
        let permit = admission.try_reserve(7, 5).unwrap();
        let control = permit.control();

        assert!(!control.is_cancelled());
        assert!(admission.cancel(7));
        assert!(control.is_cancelled());
    }

    #[test]
    fn terminal_claim_and_cancellation_have_one_winner() {
        let admission = AttachmentAdmission::for_test(5, 1);
        let permit = admission.try_reserve(7, 5).unwrap();

        assert!(permit.claim_terminal());
        assert!(admission.cancel(7));
        assert!(!permit.is_cancelled());
    }

    #[test]
    fn cancelling_all_reaches_every_active_permit() {
        let admission = AttachmentAdmission::for_test(5, 2);
        let first = admission.try_reserve(7, 2).unwrap();
        let second = admission.try_reserve(8, 3).unwrap();

        admission.cancel_all();

        assert!(first.is_cancelled());
        assert!(second.is_cancelled());
    }
}
