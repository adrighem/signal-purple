/* SPDX-License-Identifier: GPL-3.0-or-later */
#include <glib.h>

#include "signal_core.h"

static void
test_fake_core_lifecycle(void)
{
    SignalCoreConfig config = {
        .abi_version = SIGNAL_CORE_ABI_VERSION,
        .struct_size = sizeof(SignalCoreConfig),
        .store_path = "/tmp/test-fake-core",
        .device_name = "test-device",
        .passphrase = "secret",
    };
    SignalCore *core = NULL;
    SignalStatus status = signal_core_new(&config, &core);
    g_assert_cmpint(status, ==, SIGNAL_STATUS_OK);
    g_assert_nonnull(core);

    int fd = signal_core_event_fd(core);
    g_assert_cmpint(fd, >=, 0);

    SignalEvent *event = NULL;
    int poll_result = signal_core_poll_event(core, &event);
    g_assert_cmpint(poll_result, ==, 0);
    g_assert_null(event);

    status = signal_core_send_message(core, 1, "+15551234567", "hello mock");
    g_assert_cmpint(status, ==, SIGNAL_STATUS_OK);

    status = signal_core_set_typing(core, 2, "+15551234567", 1);
    g_assert_cmpint(status, ==, SIGNAL_STATUS_OK);

    status = signal_core_ack_message(core, 42);
    g_assert_cmpint(status, ==, SIGNAL_STATUS_OK);

    signal_core_shutdown(core);
    signal_core_free(core);
}

static void
test_fake_core_abi(void)
{
    g_assert_cmpuint(signal_core_abi_version(), ==, SIGNAL_CORE_ABI_VERSION);
    g_assert_cmpint(signal_core_abi_contract_value(SIGNAL_CORE_ABI_VALUE_VERSION),
                    ==, SIGNAL_CORE_ABI_VERSION);
    g_assert_cmpint(signal_core_abi_contract_value(SIGNAL_CORE_ABI_VALUE_STATUS_OK),
                    ==, SIGNAL_STATUS_OK);
}

int
main(int argc, char **argv)
{
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/signal/fake-core/lifecycle", test_fake_core_lifecycle);
    g_test_add_func("/signal/fake-core/abi", test_fake_core_abi);
    return g_test_run();
}
