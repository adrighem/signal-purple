/* SPDX-License-Identifier: GPL-3.0-or-later */
#ifndef SIGNAL_CORE_H
#define SIGNAL_CORE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define SIGNAL_CORE_ABI_VERSION 2u

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
    SIGNAL_EVENT_GROUP_MEMBER = 16
} SignalEventKind;

typedef enum {
    SIGNAL_EVENT_FLAG_NONE = 0,
    SIGNAL_EVENT_FLAG_OUTGOING = 1u << 0,
    SIGNAL_EVENT_FLAG_FATAL = 1u << 1
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

uint32_t signal_core_abi_version(void);

/*
 * Config allocations must contain at least the abi_version/struct_size prefix;
 * all string arguments must be valid NUL-terminated UTF-8. Calls for one core
 * must be serialized. Shutdown/free require exclusive access and must not race
 * polling or command submission. Every returned event must be freed once.
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

SignalStatus signal_core_set_typing(SignalCore *core,
                                    uint64_t request_id,
                                    const char *recipient,
                                    int typing);

/* Returns 1 when an event was returned, 0 when the queue is empty, or -1. */
int signal_core_poll_event(SignalCore *core, SignalEvent **out_event);

void signal_event_free(SignalEvent *event);
void signal_core_shutdown(SignalCore *core);
void signal_core_free(SignalCore *core);

#ifdef __cplusplus
}
#endif

#endif
