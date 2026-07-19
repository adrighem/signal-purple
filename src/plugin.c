/* SPDX-License-Identifier: GPL-3.0-or-later */
#include "signal_purple.h"

#ifndef SIGNAL_PURPLE_VERSION
#define SIGNAL_PURPLE_VERSION "development"
#endif

static const char *
signal_list_icon(PurpleAccount *account, PurpleBuddy *buddy)
{
    (void)account;
    (void)buddy;
    return "signal-purple";
}

static GList *
signal_status_types(PurpleAccount *account)
{
    GList *types = NULL;

    (void)account;
    types = g_list_append(types,
                          purple_status_type_new(PURPLE_STATUS_AVAILABLE,
                                                 NULL, "Connected", TRUE));
    types = g_list_append(types,
                          purple_status_type_new(PURPLE_STATUS_OFFLINE,
                                                 NULL, "Offline", TRUE));
    return types;
}

static void
signal_get_info(PurpleConnection *gc, const char *who)
{
    PurpleNotifyUserInfo *info = purple_notify_user_info_new();
    PurpleBuddy *buddy = purple_find_buddy(purple_connection_get_account(gc),
                                           who);

    purple_notify_user_info_add_pair_plaintext(info, "Signal identifier", who);
    if (buddy != NULL && purple_buddy_get_server_alias(buddy) != NULL)
        purple_notify_user_info_add_pair_plaintext(
            info, "Synced name", purple_buddy_get_server_alias(buddy));
    purple_notify_user_info_add_pair_plaintext(
        info, "Safety number",
        "Not exposed by this pre-alpha build; verify sensitive chats in an official Signal client.");
    purple_notify_userinfo(gc, who, info, NULL, NULL);
    purple_notify_user_info_destroy(info);
}

static gboolean
signal_offline_message(const PurpleBuddy *buddy)
{
    (void)buddy;
    return TRUE;
}

static const char *
signal_normalize(const PurpleAccount *account, const char *who)
{
    return purple_normalize_nocase(account, who);
}

static GHashTable *
signal_account_text_table(PurpleAccount *account)
{
    GHashTable *table = g_hash_table_new(g_str_hash, g_str_equal);

    (void)account;
    g_hash_table_insert(table, "login_label", "Local account label");
    return table;
}

static PurplePluginProtocolInfo protocol_info = {
    .options = OPT_PROTO_NO_PASSWORD,
    .user_splits = NULL,
    .protocol_options = NULL,
    .icon_spec = NO_BUDDY_ICONS,
    .list_icon = signal_list_icon,
    .status_types = signal_status_types,
    .login = signal_login,
    .close = signal_close,
    .send_im = signal_send_im,
    .send_typing = signal_send_typing,
    .get_info = signal_get_info,
    .chat_info = signal_chat_info,
    .chat_info_defaults = signal_chat_info_defaults,
    .join_chat = signal_join_chat,
    .get_chat_name = signal_get_chat_name,
    .chat_leave = signal_chat_leave,
    .chat_send = signal_chat_send,
    .normalize = signal_normalize,
    .offline_message = signal_offline_message,
    .struct_size = sizeof(PurplePluginProtocolInfo),
    .get_account_text_table = signal_account_text_table,
};

static PurplePluginInfo plugin_info = {
    .magic = PURPLE_PLUGIN_MAGIC,
    .major_version = PURPLE_MAJOR_VERSION,
    .minor_version = PURPLE_MINOR_VERSION,
    .type = PURPLE_PLUGIN_PROTOCOL,
    .priority = PURPLE_PRIORITY_DEFAULT,
    .id = SIGNAL_PLUGIN_ID,
    .name = "Signal",
    .version = SIGNAL_PURPLE_VERSION,
    .summary = "Experimental Signal protocol for libpurple 2",
    .description =
        "An unofficial, linked-device Signal protocol plugin backed by Presage.",
    .author = "Vincent van Adrighem and contributors",
    .homepage = "https://github.com/adrighem/signal-purple",
    .extra_info = &protocol_info,
};

static void
signal_plugin_init(PurplePlugin *plugin)
{
    PurpleAccountOption *option;

    (void)plugin;
    option = purple_account_option_string_new("Linked device name",
                                              "device-name",
                                              "signal-purple");
    protocol_info.protocol_options =
        g_list_append(protocol_info.protocol_options, option);

    option = purple_account_option_string_new(
        "Encrypted store path (empty uses the Purple data directory)",
        "store-path", "");
    protocol_info.protocol_options =
        g_list_append(protocol_info.protocol_options, option);
}

PURPLE_INIT_PLUGIN(signal, signal_plugin_init, plugin_info)
