/* SPDX-License-Identifier: GPL-3.0-or-later */
#include <glib.h>

#include "signal_purple.h"

_Static_assert(sizeof(SignalStatus) == sizeof(int32_t),
               "SignalStatus must have a fixed-width C ABI");

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

int
main(int argc, char **argv)
{
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/signal/abi-values", test_abi_values);
    g_test_add_func("/signal/markup-to-plaintext", test_markup_to_plaintext);
    g_test_add_func("/signal/message-flags", test_message_flags);
    return g_test_run();
}
