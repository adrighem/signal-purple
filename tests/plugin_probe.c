/* SPDX-License-Identifier: GPL-3.0-or-later */
#include <glib.h>
#include <glib/gstdio.h>
#include <purple.h>
#include <sys/stat.h>

#include "signal_purple.h"

typedef struct {
    PurpleInputFunction function;
    gpointer user_data;
} InputClosure;

static gboolean
input_dispatch(GIOChannel *channel, GIOCondition condition, gpointer data)
{
    InputClosure *closure = data;
    PurpleInputCondition purple_condition = 0;

    if ((condition & (G_IO_IN | G_IO_HUP | G_IO_ERR)) != 0)
        purple_condition |= PURPLE_INPUT_READ;
    if ((condition & (G_IO_OUT | G_IO_HUP | G_IO_ERR)) != 0)
        purple_condition |= PURPLE_INPUT_WRITE;
    closure->function(closure->user_data, g_io_channel_unix_get_fd(channel),
                      purple_condition);
    return TRUE;
}

static guint
input_add(int fd, PurpleInputCondition condition, PurpleInputFunction function,
          gpointer user_data)
{
    GIOCondition gio_condition = 0;
    GIOChannel *channel;
    InputClosure *closure;
    guint source;

    if ((condition & PURPLE_INPUT_READ) != 0)
        gio_condition |= G_IO_IN | G_IO_HUP | G_IO_ERR;
    if ((condition & PURPLE_INPUT_WRITE) != 0)
        gio_condition |= G_IO_OUT | G_IO_HUP | G_IO_ERR;

    closure = g_new(InputClosure, 1);
    closure->function = function;
    closure->user_data = user_data;
    channel = g_io_channel_unix_new(fd);
    source = g_io_add_watch_full(channel, G_PRIORITY_DEFAULT, gio_condition,
                                 input_dispatch, closure, g_free);
    g_io_channel_unref(channel);
    return source;
}

static PurpleEventLoopUiOps event_loop_ops = {
    .timeout_add = g_timeout_add,
    .timeout_remove = g_source_remove,
    .input_add = input_add,
    .input_remove = g_source_remove,
    .timeout_add_seconds = g_timeout_add_seconds,
};

static GHashTable *
get_ui_info(void)
{
    static GHashTable *info;

    if (info == NULL) {
        info = g_hash_table_new(g_str_hash, g_str_equal);
        g_hash_table_insert(info, "name", "signal-purple tests");
        g_hash_table_insert(info, "version", "0");
    }
    return info;
}

static PurpleCoreUiOps core_ops = {
    .get_ui_info = get_ui_info,
};

static void
remove_tree(const char *path)
{
    GStatBuf status;

    if (g_lstat(path, &status) != 0)
        return;
    if (!S_ISDIR(status.st_mode)) {
        g_assert_cmpint(g_remove(path), ==, 0);
        return;
    }

    GDir *directory = g_dir_open(path, 0, NULL);
    g_assert_nonnull(directory);
    for (const char *name = g_dir_read_name(directory); name != NULL;
         name = g_dir_read_name(directory)) {
        g_autofree char *child = g_build_filename(path, name, NULL);
        remove_tree(child);
    }
    g_dir_close(directory);
    g_assert_cmpint(g_rmdir(path), ==, 0);
}

int
main(int argc, char **argv)
{
    PurplePlugin *plugin;
    PurplePluginProtocolInfo *protocol;
    g_autoptr(GError) error = NULL;
    g_autofree char *user_dir = NULL;

    g_assert_cmpint(argc, ==, 2);
    user_dir = g_dir_make_tmp("signal-purple-test-XXXXXX", &error);
    g_assert_no_error(error);
    purple_util_set_user_dir(user_dir);
    purple_eventloop_set_ui_ops(&event_loop_ops);
    purple_core_set_ui_ops(&core_ops);
    g_assert_true(purple_core_init("signal-purple-tests"));

    plugin = purple_plugin_probe(argv[1]);
    g_assert_nonnull(plugin);
    g_assert_cmpstr(purple_plugin_get_id(plugin), ==, SIGNAL_PLUGIN_ID);
    g_assert_cmpstr(purple_plugin_get_name(plugin), ==, "Signal");
    g_assert_cmpstr(purple_plugin_get_version(plugin), ==,
                    EXPECTED_PLUGIN_VERSION);
    g_assert_cmpint(plugin->info->type, ==, PURPLE_PLUGIN_PROTOCOL);
    g_assert_true(purple_plugin_load(plugin));

    protocol = PURPLE_PLUGIN_PROTOCOL_INFO(plugin);
    g_assert_nonnull(protocol);
    g_assert_cmpuint(protocol->struct_size, ==,
                     sizeof(PurplePluginProtocolInfo));
    g_assert_nonnull(protocol->list_icon);
    g_assert_nonnull(protocol->status_types);
    g_assert_nonnull(protocol->login);
    g_assert_nonnull(protocol->close);
    g_assert_nonnull(protocol->send_im);
    g_assert_nonnull(protocol->send_typing);
    g_assert_null(protocol->join_chat);
    g_assert_null(protocol->get_chat_name);
    g_assert_nonnull(protocol->chat_send);

    purple_core_quit();
    remove_tree(user_dir);
    return 0;
}
