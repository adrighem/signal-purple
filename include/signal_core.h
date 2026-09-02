/* SPDX-License-Identifier: GPL-3.0-or-later */
#ifndef SIGNAL_CORE_H
#define SIGNAL_CORE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define SIGNAL_CORE_ABI_VERSION 7u

/* String limits exclude the terminating NUL byte. */
#define SIGNAL_CORE_MAX_STORE_PATH_BYTES 4096u
#define SIGNAL_CORE_MAX_DEVICE_NAME_BYTES 128u
#define SIGNAL_CORE_MAX_PASSPHRASE_BYTES 4096u
#define SIGNAL_CORE_MAX_RECIPIENT_BYTES 256u
#define SIGNAL_CORE_GROUP_KEY_BYTES 64u
#define SIGNAL_CORE_MAX_MESSAGE_BYTES (64u * 1024u)
#define SIGNAL_CORE_MAX_ATTACHMENT_FILENAME_BYTES 255u
#define SIGNAL_CORE_MAX_CONTENT_TYPE_BYTES 255u
#define SIGNAL_CORE_MAX_ATTACHMENT_BYTES (25u * 1024u * 1024u)

typedef struct SignalCore SignalCore;

typedef int32_t SignalStatus;
enum {
    SIGNAL_STATUS_OK = 0,
    SIGNAL_STATUS_INVALID_ARGUMENT = -1,
    SIGNAL_STATUS_NOT_READY = -2,
    SIGNAL_STATUS_QUEUE_FULL = -3,
    SIGNAL_STATUS_INTERNAL_ERROR = -4
};

typedef enum {
    SIGNAL_EVENT_LINK_QR = 1,
    SIGNAL_EVENT_READY = 2,
    SIGNAL_EVENT_CONTACT = 3,
    SIGNAL_EVENT_GROUP = 4,
    SIGNAL_EVENT_MESSAGE = 5,
    SIGNAL_EVENT_GROUP_MESSAGE = 6,
    SIGNAL_EVENT_TYPING = 7,
    SIGNAL_EVENT_RECEIPT = 8,
    SIGNAL_EVENT_NOTICE = 9,
    SIGNAL_EVENT_ERROR = 10,
    SIGNAL_EVENT_DISCONNECTED = 11,
    SIGNAL_EVENT_CONTACT_SYNC_BEGIN = 12,
    SIGNAL_EVENT_CONTACT_SYNC_END = 13,
    SIGNAL_EVENT_GROUP_SYNC_BEGIN = 14,
    SIGNAL_EVENT_GROUP_SYNC_END = 15,
    SIGNAL_EVENT_GROUP_MEMBER = 16,
    SIGNAL_EVENT_IDENTITY_CHANGE = 17,
    SIGNAL_EVENT_IDENTITY_ACCEPTED = 18,
    SIGNAL_EVENT_ATTACHMENT = 19,
    SIGNAL_EVENT_ATTACHMENT_SENT = 20,
    SIGNAL_EVENT_GROUP_LEFT = 21,
    SIGNAL_EVENT_RECOVERING = 22,
    SIGNAL_EVENT_ACCOUNT = 23,
    SIGNAL_EVENT_SESSION_RESET = 24
} SignalEventKind;

typedef enum {
    SIGNAL_EVENT_FLAG_NONE = 0,
    SIGNAL_EVENT_FLAG_OUTGOING = 1u << 0,
    SIGNAL_EVENT_FLAG_FATAL = 1u << 1,
    SIGNAL_EVENT_FLAG_TRANSIENT = 1u << 2
} SignalEventFlags;

typedef struct {
    uint32_t abi_version;
    uint32_t struct_size;
    const char *store_path;
    const char *device_name;
    const char *passphrase;
} SignalCoreConfig;

typedef struct {
    uint32_t abi_version;
    uint32_t struct_size;
    uint32_t kind;
    uint32_t flags;
    uint64_t request_id;
    uint64_t timestamp_ms;
    int32_t value;
    const char *peer_id;
    const char *chat_id;
    const char *title;
    const char *text;
    const uint8_t *data;
    size_t data_len;
} SignalEvent;

/*
 * Indices for signal_core_abi_contract_value. The diagnostic query lets a C
 * build compare this header with the constants and layouts compiled into the
 * Rust library. It returns INT64_MIN for an unknown index and is not a
 * replacement for SIGNAL_CORE_ABI_VERSION.
 */
typedef enum {
    SIGNAL_CORE_ABI_VALUE_VERSION = 0,
    SIGNAL_CORE_ABI_VALUE_STATUS_OK,
    SIGNAL_CORE_ABI_VALUE_STATUS_INVALID_ARGUMENT,
    SIGNAL_CORE_ABI_VALUE_STATUS_NOT_READY,
    SIGNAL_CORE_ABI_VALUE_STATUS_QUEUE_FULL,
    SIGNAL_CORE_ABI_VALUE_STATUS_INTERNAL_ERROR,
    SIGNAL_CORE_ABI_VALUE_EVENT_LINK_QR,
    SIGNAL_CORE_ABI_VALUE_EVENT_READY,
    SIGNAL_CORE_ABI_VALUE_EVENT_CONTACT,
    SIGNAL_CORE_ABI_VALUE_EVENT_GROUP,
    SIGNAL_CORE_ABI_VALUE_EVENT_MESSAGE,
    SIGNAL_CORE_ABI_VALUE_EVENT_GROUP_MESSAGE,
    SIGNAL_CORE_ABI_VALUE_EVENT_TYPING,
    SIGNAL_CORE_ABI_VALUE_EVENT_RECEIPT,
    SIGNAL_CORE_ABI_VALUE_EVENT_NOTICE,
    SIGNAL_CORE_ABI_VALUE_EVENT_ERROR,
    SIGNAL_CORE_ABI_VALUE_EVENT_DISCONNECTED,
    SIGNAL_CORE_ABI_VALUE_EVENT_CONTACT_SYNC_BEGIN,
    SIGNAL_CORE_ABI_VALUE_EVENT_CONTACT_SYNC_END,
    SIGNAL_CORE_ABI_VALUE_EVENT_GROUP_SYNC_BEGIN,
    SIGNAL_CORE_ABI_VALUE_EVENT_GROUP_SYNC_END,
    SIGNAL_CORE_ABI_VALUE_EVENT_GROUP_MEMBER,
    SIGNAL_CORE_ABI_VALUE_EVENT_IDENTITY_CHANGE,
    SIGNAL_CORE_ABI_VALUE_EVENT_IDENTITY_ACCEPTED,
    SIGNAL_CORE_ABI_VALUE_EVENT_ATTACHMENT,
    SIGNAL_CORE_ABI_VALUE_EVENT_ATTACHMENT_SENT,
    SIGNAL_CORE_ABI_VALUE_EVENT_GROUP_LEFT,
    SIGNAL_CORE_ABI_VALUE_EVENT_RECOVERING,
    SIGNAL_CORE_ABI_VALUE_EVENT_ACCOUNT,
    SIGNAL_CORE_ABI_VALUE_EVENT_SESSION_RESET,
    SIGNAL_CORE_ABI_VALUE_FLAG_NONE,
    SIGNAL_CORE_ABI_VALUE_FLAG_OUTGOING,
    SIGNAL_CORE_ABI_VALUE_FLAG_FATAL,
    SIGNAL_CORE_ABI_VALUE_FLAG_TRANSIENT,
    SIGNAL_CORE_ABI_VALUE_CONFIG_SIZE,
    SIGNAL_CORE_ABI_VALUE_CONFIG_ALIGNMENT,
    SIGNAL_CORE_ABI_VALUE_CONFIG_ABI_VERSION_OFFSET,
    SIGNAL_CORE_ABI_VALUE_CONFIG_STRUCT_SIZE_OFFSET,
    SIGNAL_CORE_ABI_VALUE_CONFIG_STORE_PATH_OFFSET,
    SIGNAL_CORE_ABI_VALUE_CONFIG_DEVICE_NAME_OFFSET,
    SIGNAL_CORE_ABI_VALUE_CONFIG_PASSPHRASE_OFFSET,
    SIGNAL_CORE_ABI_VALUE_EVENT_SIZE,
    SIGNAL_CORE_ABI_VALUE_EVENT_ALIGNMENT,
    SIGNAL_CORE_ABI_VALUE_EVENT_ABI_VERSION_OFFSET,
    SIGNAL_CORE_ABI_VALUE_EVENT_STRUCT_SIZE_OFFSET,
    SIGNAL_CORE_ABI_VALUE_EVENT_KIND_OFFSET,
    SIGNAL_CORE_ABI_VALUE_EVENT_FLAGS_OFFSET,
    SIGNAL_CORE_ABI_VALUE_EVENT_REQUEST_ID_OFFSET,
    SIGNAL_CORE_ABI_VALUE_EVENT_TIMESTAMP_MS_OFFSET,
    SIGNAL_CORE_ABI_VALUE_EVENT_VALUE_OFFSET,
    SIGNAL_CORE_ABI_VALUE_EVENT_PEER_ID_OFFSET,
    SIGNAL_CORE_ABI_VALUE_EVENT_CHAT_ID_OFFSET,
    SIGNAL_CORE_ABI_VALUE_EVENT_TITLE_OFFSET,
    SIGNAL_CORE_ABI_VALUE_EVENT_TEXT_OFFSET,
    SIGNAL_CORE_ABI_VALUE_EVENT_DATA_OFFSET,
    SIGNAL_CORE_ABI_VALUE_EVENT_DATA_LEN_OFFSET,
    SIGNAL_CORE_ABI_VALUE_MAX_STORE_PATH_BYTES,
    SIGNAL_CORE_ABI_VALUE_MAX_DEVICE_NAME_BYTES,
    SIGNAL_CORE_ABI_VALUE_MAX_PASSPHRASE_BYTES,
    SIGNAL_CORE_ABI_VALUE_MAX_RECIPIENT_BYTES,
    SIGNAL_CORE_ABI_VALUE_GROUP_KEY_BYTES,
    SIGNAL_CORE_ABI_VALUE_MAX_MESSAGE_BYTES,
    SIGNAL_CORE_ABI_VALUE_MAX_ATTACHMENT_FILENAME_BYTES,
    SIGNAL_CORE_ABI_VALUE_MAX_CONTENT_TYPE_BYTES,
    SIGNAL_CORE_ABI_VALUE_MAX_ATTACHMENT_BYTES,
    SIGNAL_CORE_ABI_VALUE_COUNT
} SignalCoreAbiValue;

uint32_t signal_core_abi_version(void);
int64_t signal_core_abi_contract_value(uint32_t index);

/*
 * Config allocations must contain at least the abi_version/struct_size prefix;
 * all string arguments must be valid NUL-terminated UTF-8. Calls for one core
 * must be serialized. Shutdown/free require exclusive access and must not race
 * polling or command submission. Input strings and attachment bytes are
 * borrowed only for the call and copied before an accepted command returns.
 *
 * signal_core_new clears out_core before validation and transfers one owned
 * core only on success. signal_core_poll_event likewise clears out_event
 * before polling. Every returned event is one Rust-owned allocation which
 * must be freed once. Its string and data fields borrow from that allocation
 * and remain valid only until signal_event_free.
 */
SignalStatus signal_core_new(const SignalCoreConfig *config,
                             SignalCore **out_core);

SignalStatus signal_core_send_message(SignalCore *core,
                                      uint64_t request_id,
                                      const char *recipient,
                                      const char *message);

SignalStatus signal_core_send_group_message(SignalCore *core,
                                            uint64_t request_id,
                                            const char *group_key,
                                            const char *message);

SignalStatus signal_core_leave_group(SignalCore *core,
                                     uint64_t request_id,
                                     const char *group_key);

SignalStatus signal_core_send_attachment(SignalCore *core,
                                         uint64_t request_id,
                                         const char *recipient,
                                         const char *filename,
                                         const char *content_type,
                                         const uint8_t *data,
                                         size_t data_len);

SignalStatus signal_core_send_group_attachment(SignalCore *core,
                                               uint64_t request_id,
                                               const char *group_key,
                                               const char *filename,
                                               const char *content_type,
                                               const uint8_t *data,
                                               size_t data_len);

SignalStatus signal_core_cancel_attachment(SignalCore *core,
                                           uint64_t request_id);

SignalStatus signal_core_set_typing(SignalCore *core,
                                    uint64_t request_id,
                                    const char *recipient,
                                    int typing);

/* Acknowledges that Purple accepted a message event for display. */
SignalStatus signal_core_ack_message(SignalCore *core,
                                     uint64_t delivery_id);

SignalStatus signal_core_accept_identity(SignalCore *core,
                                         uint64_t request_id,
                                         const char *recipient);

SignalStatus signal_core_dismiss_identity(SignalCore *core,
                                          uint64_t request_id,
                                          const char *recipient);

SignalStatus signal_core_reset_session(SignalCore *core,
                                       uint64_t request_id,
                                       const char *recipient);

SignalStatus signal_core_mark_read(SignalCore *core,
                                   uint64_t request_id,
                                   const char *recipient,
                                   uint64_t timestamp);

/* Returns a borrowed nonblocking event notifier descriptor, or -1.
 * The descriptor remains owned by the core and must not be closed by C. */
int signal_core_event_fd(SignalCore *core);

/* Returns 1 when an event was returned, 0 when the queue is empty, or -1. */
int signal_core_poll_event(SignalCore *core, SignalEvent **out_event);

void signal_event_free(SignalEvent *event);
void signal_core_shutdown(SignalCore *core);
void signal_core_free(SignalCore *core);

#ifdef __cplusplus
}
#endif

#endif
