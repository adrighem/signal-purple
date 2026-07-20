/* SPDX-License-Identifier: GPL-3.0-or-later */
#include "blist_sync.h"

static gboolean
signal_blist_sync_is_managed(PurpleBlistNode *node, const char *key)
{
    return purple_blist_node_get_bool(node, key);
}

static gboolean
signal_blist_sync_group_is(PurpleGroup *group, const char *name)
{
    return group != NULL &&
           g_strcmp0(purple_group_get_name(group), name) == 0;
}

static void
signal_blist_sync_remove_empty_legacy_group(const char *legacy_name)
{
    PurpleGroup *group = purple_find_group(legacy_name);

    if (group != NULL &&
        purple_blist_node_get_first_child(PURPLE_BLIST_NODE(group)) == NULL)
        purple_blist_remove_group(group);
}

void
signal_blist_sync_add_buddy(PurpleBuddy *buddy)
{
    g_return_if_fail(buddy != NULL);

    /* Let libpurple select its localized default buddy group. */
    purple_blist_add_buddy(buddy, NULL, NULL, NULL);
    purple_blist_node_set_bool(PURPLE_BLIST_NODE(buddy),
                               SIGNAL_SYNCED_BUDDY_KEY, TRUE);
}

PurpleBuddy *
signal_blist_sync_find_buddy(PurpleAccount *account, const char *name)
{
    GSList *buddies;
    PurpleBuddy *managed = NULL;
    PurpleBuddy *managed_legacy = NULL;
    PurpleBuddy *unmanaged_legacy = NULL;
    PurpleBuddy *fallback;

    g_return_val_if_fail(account != NULL, NULL);
    g_return_val_if_fail(name != NULL && name[0] != '\0', NULL);

    fallback = purple_find_buddy(account, name);
    buddies = purple_find_buddies(account, name);
    for (GSList *item = buddies; item != NULL; item = item->next) {
        PurpleBuddy *candidate = item->data;
        gboolean candidate_managed = signal_blist_sync_is_managed(
            PURPLE_BLIST_NODE(candidate), SIGNAL_SYNCED_BUDDY_KEY);
        gboolean candidate_legacy = signal_blist_sync_group_is(
            purple_buddy_get_group(candidate), SIGNAL_LEGACY_BUDDY_GROUP);

        if (candidate_managed && candidate_legacy) {
            managed_legacy = candidate;
            break;
        }
        if (candidate_managed && managed == NULL)
            managed = candidate;
        else if (!candidate_managed && candidate_legacy &&
                 unmanaged_legacy == NULL)
            unmanaged_legacy = candidate;
    }
    g_slist_free(buddies);

    if (managed_legacy != NULL)
        return managed_legacy;
    if (managed != NULL)
        return managed;
    if (unmanaged_legacy != NULL)
        return unmanaged_legacy;
    return fallback;
}

PurpleBuddy *
signal_blist_sync_adopt_legacy_buddy(PurpleAccount *account,
                                     const char *name)
{
    PurpleBuddy *buddy;

    g_return_val_if_fail(account != NULL, NULL);
    g_return_val_if_fail(name != NULL && name[0] != '\0', NULL);

    buddy = signal_blist_sync_find_buddy(account, name);
    if (buddy == NULL)
        return NULL;

    if (!signal_blist_sync_is_managed(PURPLE_BLIST_NODE(buddy),
                                      SIGNAL_SYNCED_BUDDY_KEY) &&
        purple_buddy_get_account(buddy) == account &&
        signal_blist_sync_group_is(purple_buddy_get_group(buddy),
                                   SIGNAL_LEGACY_BUDDY_GROUP))
        purple_blist_node_set_bool(PURPLE_BLIST_NODE(buddy),
                                   SIGNAL_SYNCED_BUDDY_KEY, TRUE);

    return signal_blist_sync_migrate_buddy(buddy);
}

PurpleBuddy *
signal_blist_sync_migrate_buddy(PurpleBuddy *buddy)
{
    PurpleAccount *account;
    PurpleContact *contact;
    PurpleGroup *legacy_group;
    g_autofree char *name = NULL;

    g_return_val_if_fail(buddy != NULL, NULL);

    legacy_group = purple_buddy_get_group(buddy);
    if (!signal_blist_sync_is_managed(PURPLE_BLIST_NODE(buddy),
                                      SIGNAL_SYNCED_BUDDY_KEY) ||
        !signal_blist_sync_group_is(legacy_group,
                                    SIGNAL_LEGACY_BUDDY_GROUP))
        return buddy;

    account = purple_buddy_get_account(buddy);
    name = g_strdup(purple_buddy_get_name(buddy));
    contact = purple_buddy_get_contact(buddy);

    /* Moving the contact keeps its alias and any locally merged buddies.
     * libpurple may destroy a duplicate buddy during this call, so do not
     * dereference buddy or contact afterwards. */
    purple_blist_add_contact(contact, NULL, NULL);
    signal_blist_sync_remove_empty_legacy_buddy_group();

    return signal_blist_sync_find_buddy(account, name);
}

gboolean
signal_blist_sync_is_legacy_buddy_group(PurpleGroup *group)
{
    return signal_blist_sync_group_is(group, SIGNAL_LEGACY_BUDDY_GROUP);
}

void
signal_blist_sync_remove_empty_legacy_buddy_group(void)
{
    signal_blist_sync_remove_empty_legacy_group(SIGNAL_LEGACY_BUDDY_GROUP);
}

void
signal_blist_sync_add_chat(PurpleChat *chat)
{
    g_return_if_fail(chat != NULL);

    /* Let libpurple select its localized default chat group. */
    purple_blist_add_chat(chat, NULL, NULL);
    purple_blist_node_set_bool(PURPLE_BLIST_NODE(chat),
                               SIGNAL_SYNCED_GROUP_KEY, TRUE);
}

void
signal_blist_sync_migrate_chat(PurpleChat *chat)
{
    PurpleGroup *legacy_group;

    g_return_if_fail(chat != NULL);

    legacy_group = purple_chat_get_group(chat);
    if (!signal_blist_sync_is_managed(PURPLE_BLIST_NODE(chat),
                                      SIGNAL_SYNCED_GROUP_KEY) ||
        !signal_blist_sync_group_is(legacy_group,
                                    SIGNAL_LEGACY_CHAT_GROUP))
        return;

    purple_blist_add_chat(chat, NULL, NULL);
    signal_blist_sync_remove_empty_legacy_chat_group();
}

gboolean
signal_blist_sync_is_legacy_chat_group(PurpleGroup *group)
{
    return signal_blist_sync_group_is(group, SIGNAL_LEGACY_CHAT_GROUP);
}

void
signal_blist_sync_remove_empty_legacy_chat_group(void)
{
    signal_blist_sync_remove_empty_legacy_group(SIGNAL_LEGACY_CHAT_GROUP);
}
