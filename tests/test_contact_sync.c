/* SPDX-License-Identifier: GPL-3.0-or-later */
#include <glib.h>

#include "contact_sync.h"

static void
test_snapshot_reconciliation(void)
{
    SignalContactSync sync = {0};

    signal_contact_sync_init(&sync);
    g_assert_false(signal_contact_sync_should_remove(&sync, "aci:contact-one"));

    signal_contact_sync_begin(&sync);
    signal_contact_sync_mark(&sync, "aci:contact-one");
    signal_contact_sync_mark(&sync, "aci:contact-two");

    g_assert_false(signal_contact_sync_should_remove(&sync, "aci:contact-one"));
    g_assert_false(signal_contact_sync_should_remove(&sync, "aci:contact-two"));
    g_assert_true(signal_contact_sync_should_remove(&sync, "aci:stale-contact"));

    signal_contact_sync_end(&sync);
    g_assert_false(signal_contact_sync_should_remove(&sync, "aci:stale-contact"));

    signal_contact_sync_begin(&sync);
    g_assert_true(signal_contact_sync_should_remove(&sync, "aci:contact-one"));
    signal_contact_sync_clear(&sync);
}

static void
test_ignores_invalid_marks(void)
{
    SignalContactSync sync = {0};

    signal_contact_sync_init(&sync);
    signal_contact_sync_mark(&sync, "aci:outside-snapshot");
    signal_contact_sync_begin(&sync);
    signal_contact_sync_mark(&sync, NULL);
    signal_contact_sync_mark(&sync, "");

    g_assert_true(
        signal_contact_sync_should_remove(&sync, "aci:outside-snapshot"));
    g_assert_false(signal_contact_sync_should_remove(&sync, NULL));
    g_assert_false(signal_contact_sync_should_remove(&sync, ""));
    signal_contact_sync_clear(&sync);
}

int
main(int argc, char **argv)
{
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/signal/contact-sync/reconciliation",
                    test_snapshot_reconciliation);
    g_test_add_func("/signal/contact-sync/invalid-marks",
                    test_ignores_invalid_marks);
    return g_test_run();
}
