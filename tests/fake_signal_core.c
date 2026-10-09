/* SPDX-License-Identifier: GPL-3.0-or-later */
#include "signal_core.h"

#include <errno.h>
#include <fcntl.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

struct SignalCore {
    int read_fd;
    int write_fd;
    int is_shutdown;
};

static const int64_t fake_abi_contract_values[SIGNAL_CORE_ABI_VALUE_COUNT] = {
    [SIGNAL_CORE_ABI_VALUE_VERSION] = SIGNAL_CORE_ABI_VERSION,
    [SIGNAL_CORE_ABI_VALUE_STATUS_OK] = SIGNAL_STATUS_OK,
    [SIGNAL_CORE_ABI_VALUE_STATUS_INVALID_ARGUMENT] =
        SIGNAL_STATUS_INVALID_ARGUMENT,
    [SIGNAL_CORE_ABI_VALUE_STATUS_NOT_READY] = SIGNAL_STATUS_NOT_READY,
    [SIGNAL_CORE_ABI_VALUE_STATUS_QUEUE_FULL] = SIGNAL_STATUS_QUEUE_FULL,
    [SIGNAL_CORE_ABI_VALUE_STATUS_INTERNAL_ERROR] =
        SIGNAL_STATUS_INTERNAL_ERROR,
    [SIGNAL_CORE_ABI_VALUE_EVENT_LINK_QR] = SIGNAL_EVENT_LINK_QR,
    [SIGNAL_CORE_ABI_VALUE_EVENT_READY] = SIGNAL_EVENT_READY,
    [SIGNAL_CORE_ABI_VALUE_EVENT_CONTACT] = SIGNAL_EVENT_CONTACT,
    [SIGNAL_CORE_ABI_VALUE_EVENT_GROUP] = SIGNAL_EVENT_GROUP,
    [SIGNAL_CORE_ABI_VALUE_EVENT_MESSAGE] = SIGNAL_EVENT_MESSAGE,
    [SIGNAL_CORE_ABI_VALUE_EVENT_GROUP_MESSAGE] = SIGNAL_EVENT_GROUP_MESSAGE,
    [SIGNAL_CORE_ABI_VALUE_EVENT_TYPING] = SIGNAL_EVENT_TYPING,
    [SIGNAL_CORE_ABI_VALUE_EVENT_RECEIPT] = SIGNAL_EVENT_RECEIPT,
    [SIGNAL_CORE_ABI_VALUE_EVENT_NOTICE] = SIGNAL_EVENT_NOTICE,
    [SIGNAL_CORE_ABI_VALUE_EVENT_ERROR] = SIGNAL_EVENT_ERROR,
    [SIGNAL_CORE_ABI_VALUE_EVENT_DISCONNECTED] = SIGNAL_EVENT_DISCONNECTED,
    [SIGNAL_CORE_ABI_VALUE_EVENT_CONTACT_SYNC_BEGIN] =
        SIGNAL_EVENT_CONTACT_SYNC_BEGIN,
    [SIGNAL_CORE_ABI_VALUE_EVENT_CONTACT_SYNC_END] =
        SIGNAL_EVENT_CONTACT_SYNC_END,
    [SIGNAL_CORE_ABI_VALUE_EVENT_GROUP_SYNC_BEGIN] =
        SIGNAL_EVENT_GROUP_SYNC_BEGIN,
    [SIGNAL_CORE_ABI_VALUE_EVENT_GROUP_SYNC_END] =
        SIGNAL_EVENT_GROUP_SYNC_END,
    [SIGNAL_CORE_ABI_VALUE_EVENT_GROUP_MEMBER] = SIGNAL_EVENT_GROUP_MEMBER,
    [SIGNAL_CORE_ABI_VALUE_EVENT_IDENTITY_CHANGE] =
        SIGNAL_EVENT_IDENTITY_CHANGE,
    [SIGNAL_CORE_ABI_VALUE_EVENT_IDENTITY_ACCEPTED] =
        SIGNAL_EVENT_IDENTITY_ACCEPTED,
    [SIGNAL_CORE_ABI_VALUE_EVENT_ATTACHMENT] = SIGNAL_EVENT_ATTACHMENT,
    [SIGNAL_CORE_ABI_VALUE_EVENT_ATTACHMENT_SENT] =
        SIGNAL_EVENT_ATTACHMENT_SENT,
    [SIGNAL_CORE_ABI_VALUE_EVENT_GROUP_LEFT] = SIGNAL_EVENT_GROUP_LEFT,
    [SIGNAL_CORE_ABI_VALUE_EVENT_RECOVERING] = SIGNAL_EVENT_RECOVERING,
    [SIGNAL_CORE_ABI_VALUE_EVENT_ACCOUNT] = SIGNAL_EVENT_ACCOUNT,
    [SIGNAL_CORE_ABI_VALUE_EVENT_SESSION_RESET] = SIGNAL_EVENT_SESSION_RESET,
    [SIGNAL_CORE_ABI_VALUE_EVENT_AVATAR] = SIGNAL_EVENT_AVATAR,
    [SIGNAL_CORE_ABI_VALUE_FLAG_NONE] = SIGNAL_EVENT_FLAG_NONE,
    [SIGNAL_CORE_ABI_VALUE_FLAG_OUTGOING] = SIGNAL_EVENT_FLAG_OUTGOING,
    [SIGNAL_CORE_ABI_VALUE_FLAG_FATAL] = SIGNAL_EVENT_FLAG_FATAL,
    [SIGNAL_CORE_ABI_VALUE_FLAG_TRANSIENT] = SIGNAL_EVENT_FLAG_TRANSIENT,
    [SIGNAL_CORE_ABI_VALUE_CONFIG_SIZE] = (int64_t)sizeof(SignalCoreConfig),
    [SIGNAL_CORE_ABI_VALUE_CONFIG_ALIGNMENT] = (int64_t)_Alignof(SignalCoreConfig),
    [SIGNAL_CORE_ABI_VALUE_CONFIG_ABI_VERSION_OFFSET] =
        (int64_t)__builtin_offsetof(SignalCoreConfig, abi_version),
    [SIGNAL_CORE_ABI_VALUE_CONFIG_STRUCT_SIZE_OFFSET] =
        (int64_t)__builtin_offsetof(SignalCoreConfig, struct_size),
    [SIGNAL_CORE_ABI_VALUE_CONFIG_STORE_PATH_OFFSET] =
        (int64_t)__builtin_offsetof(SignalCoreConfig, store_path),
    [SIGNAL_CORE_ABI_VALUE_CONFIG_DEVICE_NAME_OFFSET] =
        (int64_t)__builtin_offsetof(SignalCoreConfig, device_name),
    [SIGNAL_CORE_ABI_VALUE_CONFIG_PASSPHRASE_OFFSET] =
        (int64_t)__builtin_offsetof(SignalCoreConfig, passphrase),
    [SIGNAL_CORE_ABI_VALUE_EVENT_SIZE] = (int64_t)sizeof(SignalEvent),
    [SIGNAL_CORE_ABI_VALUE_EVENT_ALIGNMENT] = (int64_t)_Alignof(SignalEvent),
    [SIGNAL_CORE_ABI_VALUE_EVENT_ABI_VERSION_OFFSET] =
        (int64_t)__builtin_offsetof(SignalEvent, abi_version),
    [SIGNAL_CORE_ABI_VALUE_EVENT_STRUCT_SIZE_OFFSET] =
        (int64_t)__builtin_offsetof(SignalEvent, struct_size),
    [SIGNAL_CORE_ABI_VALUE_EVENT_KIND_OFFSET] =
        (int64_t)__builtin_offsetof(SignalEvent, kind),
    [SIGNAL_CORE_ABI_VALUE_EVENT_FLAGS_OFFSET] =
        (int64_t)__builtin_offsetof(SignalEvent, flags),
    [SIGNAL_CORE_ABI_VALUE_EVENT_REQUEST_ID_OFFSET] =
        (int64_t)__builtin_offsetof(SignalEvent, request_id),
    [SIGNAL_CORE_ABI_VALUE_EVENT_TIMESTAMP_MS_OFFSET] =
        (int64_t)__builtin_offsetof(SignalEvent, timestamp_ms),
    [SIGNAL_CORE_ABI_VALUE_EVENT_VALUE_OFFSET] =
        (int64_t)__builtin_offsetof(SignalEvent, value),
    [SIGNAL_CORE_ABI_VALUE_EVENT_PEER_ID_OFFSET] =
        (int64_t)__builtin_offsetof(SignalEvent, peer_id),
    [SIGNAL_CORE_ABI_VALUE_EVENT_CHAT_ID_OFFSET] =
        (int64_t)__builtin_offsetof(SignalEvent, chat_id),
    [SIGNAL_CORE_ABI_VALUE_EVENT_TITLE_OFFSET] =
        (int64_t)__builtin_offsetof(SignalEvent, title),
    [SIGNAL_CORE_ABI_VALUE_EVENT_TEXT_OFFSET] =
        (int64_t)__builtin_offsetof(SignalEvent, text),
    [SIGNAL_CORE_ABI_VALUE_EVENT_DATA_OFFSET] =
        (int64_t)__builtin_offsetof(SignalEvent, data),
    [SIGNAL_CORE_ABI_VALUE_EVENT_DATA_LEN_OFFSET] =
        (int64_t)__builtin_offsetof(SignalEvent, data_len),
    [SIGNAL_CORE_ABI_VALUE_MAX_STORE_PATH_BYTES] =
        SIGNAL_CORE_MAX_STORE_PATH_BYTES,
    [SIGNAL_CORE_ABI_VALUE_MAX_DEVICE_NAME_BYTES] =
        SIGNAL_CORE_MAX_DEVICE_NAME_BYTES,
    [SIGNAL_CORE_ABI_VALUE_MAX_PASSPHRASE_BYTES] =
        SIGNAL_CORE_MAX_PASSPHRASE_BYTES,
    [SIGNAL_CORE_ABI_VALUE_MAX_RECIPIENT_BYTES] =
        SIGNAL_CORE_MAX_RECIPIENT_BYTES,
    [SIGNAL_CORE_ABI_VALUE_GROUP_KEY_BYTES] =
        SIGNAL_CORE_GROUP_KEY_BYTES,
    [SIGNAL_CORE_ABI_VALUE_MAX_MESSAGE_BYTES] =
        SIGNAL_CORE_MAX_MESSAGE_BYTES,
    [SIGNAL_CORE_ABI_VALUE_MAX_ATTACHMENT_FILENAME_BYTES] =
        SIGNAL_CORE_MAX_ATTACHMENT_FILENAME_BYTES,
    [SIGNAL_CORE_ABI_VALUE_MAX_CONTENT_TYPE_BYTES] =
        SIGNAL_CORE_MAX_CONTENT_TYPE_BYTES,
    [SIGNAL_CORE_ABI_VALUE_MAX_ATTACHMENT_BYTES] =
        SIGNAL_CORE_MAX_ATTACHMENT_BYTES,
};

uint32_t
signal_core_abi_version(void)
{
    return SIGNAL_CORE_ABI_VERSION;
}

int64_t
signal_core_abi_contract_value(uint32_t index)
{
    if (index >= SIGNAL_CORE_ABI_VALUE_COUNT) {
        return INT64_MIN;
    }
    return fake_abi_contract_values[index];
}

SignalStatus
signal_core_new(const SignalCoreConfig *config, SignalCore **out_core)
{
    if (!out_core) {
        return SIGNAL_STATUS_INVALID_ARGUMENT;
    }
    *out_core = NULL;
    if (!config || config->abi_version != SIGNAL_CORE_ABI_VERSION ||
        config->struct_size < sizeof(SignalCoreConfig)) {
        return SIGNAL_STATUS_INVALID_ARGUMENT;
    }

    SignalCore *core = calloc(1, sizeof(*core));
    if (!core) {
        return SIGNAL_STATUS_INTERNAL_ERROR;
    }

    int pipefd[2];
    if (pipe(pipefd) != 0) {
        free(core);
        return SIGNAL_STATUS_INTERNAL_ERROR;
    }
    fcntl(pipefd[0], F_SETFL, O_NONBLOCK);
    fcntl(pipefd[1], F_SETFL, O_NONBLOCK);
    core->read_fd = pipefd[0];
    core->write_fd = pipefd[1];
    *out_core = core;
    return SIGNAL_STATUS_OK;
}

SignalStatus
signal_core_send_message(SignalCore *core,
                         uint64_t request_id,
                         const char *recipient,
                         const char *message)
{
    (void)request_id;
    if (!core || core->is_shutdown || !recipient || !message) {
        return SIGNAL_STATUS_INVALID_ARGUMENT;
    }
    return SIGNAL_STATUS_OK;
}

SignalStatus
signal_core_send_group_message(SignalCore *core,
                               uint64_t request_id,
                               const char *group_key,
                               const char *message)
{
    (void)request_id;
    if (!core || core->is_shutdown || !group_key || !message) {
        return SIGNAL_STATUS_INVALID_ARGUMENT;
    }
    return SIGNAL_STATUS_OK;
}

SignalStatus
signal_core_leave_group(SignalCore *core,
                        uint64_t request_id,
                        const char *group_key)
{
    (void)request_id;
    if (!core || core->is_shutdown || !group_key) {
        return SIGNAL_STATUS_INVALID_ARGUMENT;
    }
    return SIGNAL_STATUS_OK;
}

SignalStatus
signal_core_send_attachment(SignalCore *core,
                            uint64_t request_id,
                            const char *recipient,
                            const char *filename,
                            const char *content_type,
                            const uint8_t *data,
                            size_t data_len)
{
    (void)request_id;
    if (!core || core->is_shutdown || !recipient || !filename ||
        !content_type || !data || data_len == 0) {
        return SIGNAL_STATUS_INVALID_ARGUMENT;
    }
    return SIGNAL_STATUS_OK;
}

SignalStatus
signal_core_send_file_attachment(SignalCore *core,
                                 uint64_t request_id,
                                 const char *recipient,
                                 const char *filename,
                                 const char *content_type,
                                 const char *file_path)
{
    (void)request_id;
    if (!core || core->is_shutdown || !recipient || !filename ||
        !content_type || !file_path) {
        return SIGNAL_STATUS_INVALID_ARGUMENT;
    }
    return SIGNAL_STATUS_OK;
}

SignalStatus
signal_core_send_group_attachment(SignalCore *core,
                                  uint64_t request_id,
                                  const char *group_key,
                                  const char *filename,
                                  const char *content_type,
                                  const uint8_t *data,
                                  size_t data_len)
{
    (void)request_id;
    if (!core || core->is_shutdown || !group_key || !filename ||
        !content_type || !data || data_len == 0) {
        return SIGNAL_STATUS_INVALID_ARGUMENT;
    }
    return SIGNAL_STATUS_OK;
}

SignalStatus
signal_core_send_group_file_attachment(SignalCore *core,
                                       uint64_t request_id,
                                       const char *group_key,
                                       const char *filename,
                                       const char *content_type,
                                       const char *file_path)
{
    (void)request_id;
    if (!core || core->is_shutdown || !group_key || !filename ||
        !content_type || !file_path) {
        return SIGNAL_STATUS_INVALID_ARGUMENT;
    }
    return SIGNAL_STATUS_OK;
}

SignalStatus
signal_core_cancel_attachment(SignalCore *core, uint64_t request_id)
{
    (void)request_id;
    if (!core || core->is_shutdown) {
        return SIGNAL_STATUS_INVALID_ARGUMENT;
    }
    return SIGNAL_STATUS_OK;
}

SignalStatus
signal_core_set_typing(SignalCore *core,
                       uint64_t request_id,
                       const char *recipient,
                       int typing)
{
    (void)request_id;
    (void)typing;
    if (!core || core->is_shutdown || !recipient) {
        return SIGNAL_STATUS_INVALID_ARGUMENT;
    }
    return SIGNAL_STATUS_OK;
}

SignalStatus
signal_core_ack_message(SignalCore *core, uint64_t delivery_id)
{
    (void)delivery_id;
    if (!core || core->is_shutdown) {
        return SIGNAL_STATUS_INVALID_ARGUMENT;
    }
    return SIGNAL_STATUS_OK;
}

SignalStatus
signal_core_accept_identity(SignalCore *core,
                            uint64_t request_id,
                            const char *recipient)
{
    (void)request_id;
    if (!core || core->is_shutdown || !recipient) {
        return SIGNAL_STATUS_INVALID_ARGUMENT;
    }
    return SIGNAL_STATUS_OK;
}

SignalStatus
signal_core_dismiss_identity(SignalCore *core,
                             uint64_t request_id,
                             const char *recipient)
{
    (void)request_id;
    if (!core || core->is_shutdown || !recipient) {
        return SIGNAL_STATUS_INVALID_ARGUMENT;
    }
    return SIGNAL_STATUS_OK;
}

SignalStatus
signal_core_reset_session(SignalCore *core,
                          uint64_t request_id,
                          const char *recipient)
{
    (void)request_id;
    if (!core || core->is_shutdown || !recipient) {
        return SIGNAL_STATUS_INVALID_ARGUMENT;
    }
    return SIGNAL_STATUS_OK;
}

SignalStatus
signal_core_mark_read(SignalCore *core,
                      uint64_t request_id,
                      const char *recipient,
                      uint64_t timestamp)
{
    (void)request_id;
    (void)timestamp;
    if (!core || core->is_shutdown || !recipient) {
        return SIGNAL_STATUS_INVALID_ARGUMENT;
    }
    return SIGNAL_STATUS_OK;
}

int
signal_core_event_fd(SignalCore *core)
{
    if (!core || core->is_shutdown) {
        return -1;
    }
    return core->read_fd;
}

int
signal_core_poll_event(SignalCore *core, SignalEvent **out_event)
{
    if (!out_event) {
        return -1;
    }
    *out_event = NULL;
    if (!core || core->is_shutdown) {
        return -1;
    }
    return 0;
}

void
signal_event_free(SignalEvent *event)
{
    free(event);
}

void
signal_core_shutdown(SignalCore *core)
{
    if (!core) {
        return;
    }
    core->is_shutdown = 1;
}

void
signal_core_free(SignalCore *core)
{
    if (!core) {
        return;
    }
    if (core->read_fd >= 0) {
        close(core->read_fd);
    }
    if (core->write_fd >= 0) {
        close(core->write_fd);
    }
    free(core);
}
