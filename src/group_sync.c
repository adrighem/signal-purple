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
    GHashTable *components;
    const char *candidate_id;

    if (purple_chat_get_account(chat) != account)
        return FALSE;
    components = purple_chat_get_components(chat);
    candidate_id = g_hash_table_lookup(components,
                                       SIGNAL_GROUP_COMPONENT_KEY);
    return g_strcmp0(candidate_id, group_id) == 0;
}

PurpleChat *
signal_group_sync_find_chat(PurpleAccount *account, const char *group_id,
                            guint *removed)
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

    node = purple_blist_get_root();
    while (node != NULL) {
        PurpleBlistNode *next = purple_blist_node_next(node, FALSE);

        if (PURPLE_BLIST_NODE_IS_CHAT(node)) {
            PurpleChat *candidate = PURPLE_CHAT(node);

            if (candidate != selected && signal_chat_is_managed(candidate) &&
                signal_chat_matches(candidate, account, group_id)) {
                purple_blist_remove_chat(candidate);
                if (removed != NULL)
                    (*removed)++;
            }
        }
        node = next;
    }
    return selected;
}
