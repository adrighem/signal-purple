/* SPDX-License-Identifier: GPL-3.0-or-later */
#include <glib.h>

#include "signal_purple.h"

_Static_assert(sizeof(SignalStatus) == sizeof(int32_t),
               "SignalStatus must have a fixed-width C ABI");

static const gint64 abi_contract_values[SIGNAL_CORE_ABI_VALUE_COUNT] = {
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
    [SIGNAL_CORE_ABI_VALUE_CONFIG_SIZE] = sizeof(SignalCoreConfig),
    [SIGNAL_CORE_ABI_VALUE_CONFIG_ALIGNMENT] = _Alignof(SignalCoreConfig),
    [SIGNAL_CORE_ABI_VALUE_CONFIG_ABI_VERSION_OFFSET] =
        offsetof(SignalCoreConfig, abi_version),
    [SIGNAL_CORE_ABI_VALUE_CONFIG_STRUCT_SIZE_OFFSET] =
        offsetof(SignalCoreConfig, struct_size),
    [SIGNAL_CORE_ABI_VALUE_CONFIG_STORE_PATH_OFFSET] =
        offsetof(SignalCoreConfig, store_path),
    [SIGNAL_CORE_ABI_VALUE_CONFIG_DEVICE_NAME_OFFSET] =
        offsetof(SignalCoreConfig, device_name),
    [SIGNAL_CORE_ABI_VALUE_CONFIG_PASSPHRASE_OFFSET] =
        offsetof(SignalCoreConfig, passphrase),
    [SIGNAL_CORE_ABI_VALUE_EVENT_SIZE] = sizeof(SignalEvent),
    [SIGNAL_CORE_ABI_VALUE_EVENT_ALIGNMENT] = _Alignof(SignalEvent),
    [SIGNAL_CORE_ABI_VALUE_EVENT_ABI_VERSION_OFFSET] =
        offsetof(SignalEvent, abi_version),
    [SIGNAL_CORE_ABI_VALUE_EVENT_STRUCT_SIZE_OFFSET] =
        offsetof(SignalEvent, struct_size),
    [SIGNAL_CORE_ABI_VALUE_EVENT_KIND_OFFSET] = offsetof(SignalEvent, kind),
    [SIGNAL_CORE_ABI_VALUE_EVENT_FLAGS_OFFSET] = offsetof(SignalEvent, flags),
    [SIGNAL_CORE_ABI_VALUE_EVENT_REQUEST_ID_OFFSET] =
        offsetof(SignalEvent, request_id),
    [SIGNAL_CORE_ABI_VALUE_EVENT_TIMESTAMP_MS_OFFSET] =
        offsetof(SignalEvent, timestamp_ms),
    [SIGNAL_CORE_ABI_VALUE_EVENT_VALUE_OFFSET] = offsetof(SignalEvent, value),
    [SIGNAL_CORE_ABI_VALUE_EVENT_PEER_ID_OFFSET] =
        offsetof(SignalEvent, peer_id),
    [SIGNAL_CORE_ABI_VALUE_EVENT_CHAT_ID_OFFSET] =
        offsetof(SignalEvent, chat_id),
    [SIGNAL_CORE_ABI_VALUE_EVENT_TITLE_OFFSET] = offsetof(SignalEvent, title),
    [SIGNAL_CORE_ABI_VALUE_EVENT_TEXT_OFFSET] = offsetof(SignalEvent, text),
    [SIGNAL_CORE_ABI_VALUE_EVENT_DATA_OFFSET] = offsetof(SignalEvent, data),
    [SIGNAL_CORE_ABI_VALUE_EVENT_DATA_LEN_OFFSET] =
        offsetof(SignalEvent, data_len),
    [SIGNAL_CORE_ABI_VALUE_MAX_STORE_PATH_BYTES] =
        SIGNAL_CORE_MAX_STORE_PATH_BYTES,
    [SIGNAL_CORE_ABI_VALUE_MAX_DEVICE_NAME_BYTES] =
        SIGNAL_CORE_MAX_DEVICE_NAME_BYTES,
    [SIGNAL_CORE_ABI_VALUE_MAX_PASSPHRASE_BYTES] =
        SIGNAL_CORE_MAX_PASSPHRASE_BYTES,
    [SIGNAL_CORE_ABI_VALUE_MAX_RECIPIENT_BYTES] =
        SIGNAL_CORE_MAX_RECIPIENT_BYTES,
    [SIGNAL_CORE_ABI_VALUE_GROUP_KEY_BYTES] = SIGNAL_CORE_GROUP_KEY_BYTES,
    [SIGNAL_CORE_ABI_VALUE_MAX_MESSAGE_BYTES] =
        SIGNAL_CORE_MAX_MESSAGE_BYTES,
    [SIGNAL_CORE_ABI_VALUE_MAX_ATTACHMENT_FILENAME_BYTES] =
        SIGNAL_CORE_MAX_ATTACHMENT_FILENAME_BYTES,
    [SIGNAL_CORE_ABI_VALUE_MAX_CONTENT_TYPE_BYTES] =
        SIGNAL_CORE_MAX_CONTENT_TYPE_BYTES,
    [SIGNAL_CORE_ABI_VALUE_MAX_ATTACHMENT_BYTES] =
        SIGNAL_CORE_MAX_ATTACHMENT_BYTES,
};

G_STATIC_ASSERT(G_N_ELEMENTS(abi_contract_values) ==
                SIGNAL_CORE_ABI_VALUE_COUNT);

static void
test_abi_values(void)
{
    const SignalStatus statuses[] = {
        SIGNAL_STATUS_OK,
        SIGNAL_STATUS_INVALID_ARGUMENT,
        SIGNAL_STATUS_NOT_READY,
        SIGNAL_STATUS_QUEUE_FULL,
        SIGNAL_STATUS_INTERNAL_ERROR,
    };
    const SignalEventKind events[] = {
        SIGNAL_EVENT_LINK_QR,
        SIGNAL_EVENT_READY,
        SIGNAL_EVENT_CONTACT,
        SIGNAL_EVENT_GROUP,
        SIGNAL_EVENT_MESSAGE,
        SIGNAL_EVENT_GROUP_MESSAGE,
        SIGNAL_EVENT_TYPING,
        SIGNAL_EVENT_RECEIPT,
        SIGNAL_EVENT_NOTICE,
        SIGNAL_EVENT_ERROR,
        SIGNAL_EVENT_DISCONNECTED,
        SIGNAL_EVENT_CONTACT_SYNC_BEGIN,
        SIGNAL_EVENT_CONTACT_SYNC_END,
        SIGNAL_EVENT_GROUP_SYNC_BEGIN,
        SIGNAL_EVENT_GROUP_SYNC_END,
        SIGNAL_EVENT_GROUP_MEMBER,
        SIGNAL_EVENT_IDENTITY_CHANGE,
        SIGNAL_EVENT_IDENTITY_ACCEPTED,
        SIGNAL_EVENT_ATTACHMENT,
        SIGNAL_EVENT_ATTACHMENT_SENT,
        SIGNAL_EVENT_GROUP_LEFT,
        SIGNAL_EVENT_RECOVERING,
        SIGNAL_EVENT_ACCOUNT,
        SIGNAL_EVENT_SESSION_RESET,
        SIGNAL_EVENT_AVATAR,
    };

    g_assert_cmpuint(SIGNAL_CORE_ABI_VERSION, ==, 7);
    for (guint index = 0; index < G_N_ELEMENTS(statuses); index++)
        g_assert_cmpint(statuses[index], ==, -(gint)index);
    for (guint index = 0; index < G_N_ELEMENTS(events); index++)
        g_assert_cmpuint(events[index], ==, index + 1);
    g_assert_cmpuint(SIGNAL_EVENT_FLAG_NONE, ==, 0);
    g_assert_cmpuint(SIGNAL_EVENT_FLAG_OUTGOING, ==, 1);
    g_assert_cmpuint(SIGNAL_EVENT_FLAG_FATAL, ==, 2);
    g_assert_cmpuint(SIGNAL_EVENT_FLAG_TRANSIENT, ==, 4);
}

static void
test_abi_contract_matches_rust(void)
{
    g_assert_cmpuint(signal_core_abi_version(), ==, SIGNAL_CORE_ABI_VERSION);

    for (guint index = 0; index < G_N_ELEMENTS(abi_contract_values); index++)
        g_assert_cmpint(signal_core_abi_contract_value(index), ==,
                        abi_contract_values[index]);

    g_assert_cmpint(signal_core_abi_contract_value(SIGNAL_CORE_ABI_VALUE_COUNT),
                    ==, G_MININT64);
}

static void
test_markup_to_plaintext(void)
{
    g_autofree char *plain = signal_plaintext_from_markup(
        "Hello <b>world</b><br>two &amp; three");

    g_assert_cmpstr(plain, ==, "Hello world\ntwo & three");
    g_assert_null(signal_plaintext_from_markup(NULL));
}

static void
test_message_flags(void)
{
    PurpleMessageFlags incoming = signal_message_flags(FALSE);
    PurpleMessageFlags outgoing = signal_message_flags(TRUE);

    g_assert_cmpuint(incoming, ==, PURPLE_MESSAGE_RECV);
    g_assert_cmpuint(outgoing, ==,
                     PURPLE_MESSAGE_SEND | PURPLE_MESSAGE_REMOTE_SEND);
    g_assert_cmpuint(incoming & (PURPLE_MESSAGE_SYSTEM | PURPLE_MESSAGE_NOTIFY |
                                 PURPLE_MESSAGE_NO_LOG),
                     ==, 0);
    g_assert_cmpuint(outgoing & (PURPLE_MESSAGE_SYSTEM | PURPLE_MESSAGE_NOTIFY |
                                 PURPLE_MESSAGE_NO_LOG),
                     ==, 0);
}

static void
test_persisted_identifiers(void)
{
    g_assert_cmpstr(SIGNAL_PLUGIN_ID, ==, "prpl-adrighem-signal");
    g_assert_cmpstr(SIGNAL_STORE_ID_KEY, ==, "store-id");
    g_assert_cmpstr(SIGNAL_STORE_PATH_KEY, ==, "store-path");
    g_assert_cmpstr(SIGNAL_DEVICE_NAME_KEY, ==, "device-name");
    g_assert_cmpstr(SIGNAL_SYNCED_BUDDY_KEY, ==, "signal-purple-synced-contact");
    g_assert_cmpstr(SIGNAL_SYNCED_GROUP_KEY, ==, "signal-purple-synced-group");
}

int
main(int argc, char **argv)
{
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/signal/abi-values", test_abi_values);
    g_test_add_func("/signal/abi-contract", test_abi_contract_matches_rust);
    g_test_add_func("/signal/markup-to-plaintext", test_markup_to_plaintext);
    g_test_add_func("/signal/message-flags", test_message_flags);
    g_test_add_func("/signal/persisted-identifiers", test_persisted_identifiers);
    return g_test_run();
}
