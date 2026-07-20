/* SPDX-License-Identifier: GPL-3.0-or-later */
#include "group_sync.h"

static gboolean
signal_chat_is_managed(PurpleChat *chat)
{
    return purple_blist_node_get_bool(PURPLE_BLIST_NODE(chat),
                                      SIGNAL_SYNCED_GROUP_KEY);
}

static gboolean
signal_chat_matches(PurpleChat *chat, PurpleAccount *account,
                    const char *group_id)
{
    if (purple_chat_get_account(chat) != account)
        return FALSE;
    return g_strcmp0(signal_group_sync_chat_id(chat), group_id) == 0;
}

const char *
signal_group_sync_chat_id(PurpleChat *chat)
{
    GHashTable *components;
    const char *group_id;

    g_return_val_if_fail(chat != NULL, NULL);

    components = purple_chat_get_components(chat);
    group_id = g_hash_table_lookup(components, SIGNAL_GROUP_COMPONENT_KEY);
    if (group_id == NULL)
        group_id = g_hash_table_lookup(components,
                                       SIGNAL_LEGACY_GROUP_COMPONENT_KEY);
    return group_id;
}

PurpleChat *
signal_group_sync_lookup_chat(PurpleAccount *account, const char *group_id)
{
    PurpleBlistNode *node;
    PurpleChat *selected = NULL;

    g_return_val_if_fail(account != NULL, NULL);
    g_return_val_if_fail(group_id != NULL && group_id[0] != '\0', NULL);

    for (node = purple_blist_get_root(); node != NULL;
         node = purple_blist_node_next(node, FALSE)) {
        PurpleChat *candidate;

        if (!PURPLE_BLIST_NODE_IS_CHAT(node))
            continue;
        candidate = PURPLE_CHAT(node);
        if (!signal_chat_matches(candidate, account, group_id))
            continue;
        if (selected == NULL ||
            (signal_chat_is_managed(selected) &&
             !signal_chat_is_managed(candidate)))
            selected = candidate;
    }

    return selected;
}

PurpleChat *
signal_group_sync_find_chat(PurpleAccount *account, const char *group_id,
                            guint *removed)
{
    PurpleBlistNode *node;
    PurpleChat *selected;

    g_return_val_if_fail(account != NULL, NULL);
    g_return_val_if_fail(group_id != NULL && group_id[0] != '\0', NULL);
    selected = signal_group_sync_lookup_chat(account, group_id);

    node = purple_blist_get_root();
    while (node != NULL) {
        PurpleBlistNode *next = purple_blist_node_next(node, FALSE);

        if (PURPLE_BLIST_NODE_IS_CHAT(node)) {
            PurpleChat *candidate = PURPLE_CHAT(node);

            if (candidate != selected && signal_chat_is_managed(candidate) &&
                signal_chat_matches(candidate, account, group_id)) {
                gboolean was_legacy = signal_blist_sync_is_legacy_chat_group(
                    purple_chat_get_group(candidate));

                purple_blist_remove_chat(candidate);
                if (was_legacy)
                    signal_blist_sync_remove_empty_legacy_chat_group();
                if (removed != NULL)
                    (*removed)++;
            }
        }
        node = next;
    }
    return selected;
}

guint
signal_group_sync_remove_managed_chats(PurpleAccount *account,
                                       const char *group_id)
{
    PurpleBlistNode *node;
    guint removed = 0;

    g_return_val_if_fail(account != NULL, 0);
    g_return_val_if_fail(group_id != NULL && group_id[0] != '\0', 0);

    node = purple_blist_get_root();
    while (node != NULL) {
        PurpleBlistNode *next = purple_blist_node_next(node, FALSE);

        if (PURPLE_BLIST_NODE_IS_CHAT(node)) {
            PurpleChat *chat = PURPLE_CHAT(node);

            if (signal_chat_is_managed(chat) &&
                signal_chat_matches(chat, account, group_id)) {
                gboolean was_legacy = signal_blist_sync_is_legacy_chat_group(
                    purple_chat_get_group(chat));

                purple_blist_remove_chat(chat);
                if (was_legacy)
                    signal_blist_sync_remove_empty_legacy_chat_group();
                removed++;
            }
        }
        node = next;
    }

    return removed;
}

void
signal_group_sync_update_title(PurpleChat *chat, const char *group_id,
                               const char *title)
{
    PurpleBlistNode *node;
    const char *display_title;
    const char *previous_title;
    gboolean follows_signal_title;

    g_return_if_fail(chat != NULL);
    g_return_if_fail(group_id != NULL && group_id[0] != '\0');
    g_return_if_fail(title != NULL && title[0] != '\0');

    node = PURPLE_BLIST_NODE(chat);
    display_title = purple_chat_get_name(chat);
    previous_title = purple_blist_node_get_string(
        node, SIGNAL_SYNCED_GROUP_TITLE_KEY);
    follows_signal_title =
        display_title == NULL || display_title[0] == '\0' ||
        g_strcmp0(display_title, group_id) == 0 ||
        (previous_title != NULL &&
         g_strcmp0(display_title, previous_title) == 0) ||
        (previous_title == NULL && g_strcmp0(display_title, title) == 0);

    if (follows_signal_title)
        purple_blist_alias_chat(chat, title);
    purple_blist_node_set_string(node, SIGNAL_SYNCED_GROUP_TITLE_KEY, title);
}
