// SPDX-License-Identifier: AGPL-3.0-only
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

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
    request_ids: HashSet<u64>,
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
                request_ids: HashSet::new(),
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
        if state.request_ids.contains(&request_id) {
            return Err(AttachmentAdmissionError::Invalid);
        }
        if state.request_ids.len() >= self.maximum_attachments
            || state
                .bytes
                .checked_add(bytes)
                .is_none_or(|total| total > self.maximum_bytes)
        {
            return Err(AttachmentAdmissionError::Capacity);
        }

        state.bytes += bytes;
        state.request_ids.insert(request_id);
        Ok(AttachmentPermit {
            admission: Arc::clone(self),
            request_id,
            bytes,
        })
    }

    fn release(&self, request_id: u64, bytes: usize) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.request_ids.remove(&request_id) {
            state.bytes = state.bytes.saturating_sub(bytes);
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(maximum_bytes: usize, maximum_attachments: usize) -> Arc<Self> {
        Self::with_limits(maximum_bytes, maximum_attachments)
    }

    #[cfg(test)]
    pub(crate) fn usage(&self) -> (usize, usize) {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        (state.bytes, state.request_ids.len())
    }
}

#[derive(Debug)]
pub(crate) struct AttachmentPermit {
    admission: Arc<AttachmentAdmission>,
    request_id: u64,
    bytes: usize,
}

impl Drop for AttachmentPermit {
    fn drop(&mut self) {
        self.admission.release(self.request_id, self.bytes);
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
}
