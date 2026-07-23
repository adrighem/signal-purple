// SPDX-License-Identifier: AGPL-3.0-only
use std::ffi::{CString, c_char};

pub const ABI_VERSION: u32 = 7;

pub const EVENT_LINK_QR: u32 = 1;
pub const EVENT_READY: u32 = 2;
pub const EVENT_CONTACT: u32 = 3;
pub const EVENT_GROUP: u32 = 4;
pub const EVENT_MESSAGE: u32 = 5;
pub const EVENT_GROUP_MESSAGE: u32 = 6;
pub const EVENT_TYPING: u32 = 7;
pub const EVENT_RECEIPT: u32 = 8;
pub const EVENT_ERROR: u32 = 10;
pub const EVENT_DISCONNECTED: u32 = 11;
pub const EVENT_CONTACT_SYNC_BEGIN: u32 = 12;
pub const EVENT_CONTACT_SYNC_END: u32 = 13;
pub const EVENT_GROUP_SYNC_BEGIN: u32 = 14;
pub const EVENT_GROUP_SYNC_END: u32 = 15;
pub const EVENT_GROUP_MEMBER: u32 = 16;
pub const EVENT_IDENTITY_CHANGE: u32 = 17;
pub const EVENT_IDENTITY_ACCEPTED: u32 = 18;
pub const EVENT_ATTACHMENT: u32 = 19;
pub const EVENT_ATTACHMENT_SENT: u32 = 20;
pub const EVENT_GROUP_LEFT: u32 = 21;
pub const EVENT_RECOVERING: u32 = 22;

pub const FLAG_OUTGOING: u32 = 1 << 0;
pub const FLAG_FATAL: u32 = 1 << 1;
pub const FLAG_TRANSIENT: u32 = 1 << 2;

#[derive(Debug, Default)]
pub struct Event {
    pub kind: u32,
    pub flags: u32,
    pub request_id: u64,
    pub timestamp_ms: u64,
    pub value: i32,
    pub peer_id: Option<String>,
    pub chat_id: Option<String>,
    pub title: Option<String>,
    pub text: Option<String>,
    pub data: Vec<u8>,
}

impl Event {
    pub fn error(message: impl Into<String>, fatal: bool) -> Self {
        Self {
            kind: EVENT_ERROR,
            flags: if fatal { FLAG_FATAL } else { 0 },
            title: Some(if fatal {
                "Signal backend stopped".into()
            } else {
                "Signal operation failed".into()
            }),
            text: Some(message.into()),
            ..Self::default()
        }
    }

    pub fn request_error(request_id: u64, message: impl Into<String>) -> Self {
        Self {
            request_id,
            ..Self::error(message, false)
        }
    }

    pub fn transient_error(message: impl Into<String>) -> Self {
        Self {
            flags: FLAG_TRANSIENT,
            ..Self::error(message, false)
        }
    }

    pub fn group_request_error(
        request_id: u64,
        chat_id: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            request_id,
            chat_id: Some(chat_id.into()),
            ..Self::error(message, false)
        }
    }
}

#[repr(C)]
pub struct SignalEvent {
    pub abi_version: u32,
    pub struct_size: u32,
    pub kind: u32,
    pub flags: u32,
    pub request_id: u64,
    pub timestamp_ms: u64,
    pub value: i32,
    pub peer_id: *const c_char,
    pub chat_id: *const c_char,
    pub title: *const c_char,
    pub text: *const c_char,
    pub data: *const u8,
    pub data_len: usize,
}

#[repr(C)]
pub struct OwnedEvent {
    pub public: SignalEvent,
    peer_id: Option<CString>,
    chat_id: Option<CString>,
    title: Option<CString>,
    text: Option<CString>,
    data: Vec<u8>,
}

fn c_string(value: Option<String>) -> Option<CString> {
    value.map(|value| {
        CString::new(value.replace('\0', "�")).expect("replacement removes embedded NUL bytes")
    })
}

impl OwnedEvent {
    pub fn new(event: Event) -> Box<Self> {
        let mut owned = Box::new(Self {
            public: SignalEvent {
                abi_version: ABI_VERSION,
                struct_size: size_of::<SignalEvent>() as u32,
                kind: event.kind,
                flags: event.flags,
                request_id: event.request_id,
                timestamp_ms: event.timestamp_ms,
                value: event.value,
                peer_id: std::ptr::null(),
                chat_id: std::ptr::null(),
                title: std::ptr::null(),
                text: std::ptr::null(),
                data: std::ptr::null(),
                data_len: event.data.len(),
            },
            peer_id: c_string(event.peer_id),
            chat_id: c_string(event.chat_id),
            title: c_string(event.title),
            text: c_string(event.text),
            data: event.data,
        });

        owned.public.peer_id = owned
            .peer_id
            .as_ref()
            .map_or(std::ptr::null(), |value| value.as_ptr());
        owned.public.chat_id = owned
            .chat_id
            .as_ref()
            .map_or(std::ptr::null(), |value| value.as_ptr());
        owned.public.title = owned
            .title
            .as_ref()
            .map_or(std::ptr::null(), |value| value.as_ptr());
        owned.public.text = owned
            .text
            .as_ref()
            .map_or(std::ptr::null(), |value| value.as_ptr());
        owned.public.data = if owned.data.is_empty() {
            std::ptr::null()
        } else {
            owned.data.as_ptr()
        };
        owned
    }

    pub fn into_raw(event: Event) -> *mut SignalEvent {
        let owned = Self::new(event);
        Box::into_raw(owned).cast::<SignalEvent>()
    }

    pub unsafe fn free(event: *mut SignalEvent) {
        if !event.is_null() {
            // SAFETY: `event` was returned by `OwnedEvent::into_raw`, and
            // `public` is the first repr(C) field of `OwnedEvent`.
            drop(unsafe { Box::from_raw(event.cast::<OwnedEvent>()) });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn owns_and_sanitizes_event_payloads() {
        let raw = OwnedEvent::into_raw(Event {
            kind: EVENT_MESSAGE,
            peer_id: Some("aci:test".into()),
            text: Some("hello\0world".into()),
            data: vec![1, 2, 3],
            ..Event::default()
        });

        // SAFETY: this test owns `raw` until the matching free below.
        unsafe {
            assert_eq!((*raw).abi_version, ABI_VERSION);
            assert_eq!(CStr::from_ptr((*raw).peer_id).to_str().unwrap(), "aci:test");
            assert_eq!(CStr::from_ptr((*raw).text).to_str().unwrap(), "hello�world");
            assert_eq!(
                std::slice::from_raw_parts((*raw).data, (*raw).data_len),
                [1, 2, 3]
            );
            OwnedEvent::free(raw);
        }
    }

    #[test]
    fn group_request_errors_preserve_request_and_chat_identity() {
        let event = Event::group_request_error(17, "opaque-group-id", "leave failed");

        assert_eq!(event.kind, EVENT_ERROR);
        assert_eq!(event.request_id, 17);
        assert_eq!(event.chat_id.as_deref(), Some("opaque-group-id"));
        assert_eq!(event.text.as_deref(), Some("leave failed"));
    }

    #[test]
    fn transient_errors_are_nonfatal_and_explicitly_classified() {
        let event = Event::transient_error("network is recovering");

        assert_eq!(event.kind, EVENT_ERROR);
        assert_eq!(event.flags, FLAG_TRANSIENT);
        assert_eq!(event.title.as_deref(), Some("Signal operation failed"));
        assert_eq!(event.text.as_deref(), Some("network is recovering"));
    }
}
