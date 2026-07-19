/* SPDX-License-Identifier: GPL-3.0-or-later */
#include <glib.h>

#include "signal_purple.h"

_Static_assert(sizeof(SignalStatus) == sizeof(int32_t),
               "SignalStatus must have a fixed-width C ABI");

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
    g_test_add_func("/signal/markup-to-plaintext", test_markup_to_plaintext);
    g_test_add_func("/signal/message-flags", test_message_flags);
    return g_test_run();
}
