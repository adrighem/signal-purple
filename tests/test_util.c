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

int
main(int argc, char **argv)
{
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/signal/markup-to-plaintext", test_markup_to_plaintext);
    return g_test_run();
}
