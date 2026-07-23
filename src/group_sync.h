/* SPDX-License-Identifier: GPL-3.0-or-later */
#ifndef SIGNAL_GROUP_SYNC_H
#define SIGNAL_GROUP_SYNC_H

#include <purple.h>

#include "blist_sync.h"

#define SIGNAL_GROUP_COMPONENT_KEY "group-id"
#define SIGNAL_LEGACY_GROUP_COMPONENT_KEY "group-key"
#define SIGNAL_SYNCED_GROUP_TITLE_KEY "signal-purple-synced-group-title"

PurpleChat *signal_group_sync_lookup_chat(PurpleAccount *account,
                                          const char *group_id);
PurpleConversation *signal_group_sync_lookup_conversation(
    PurpleAccount *account, const char *group_id);
PurpleChat *signal_group_sync_find_chat(PurpleAccount *account,
                                        const char *group_id,
                                        guint *removed);
const char *signal_group_sync_chat_id(PurpleChat *chat);
guint signal_group_sync_remove_managed_chats(PurpleAccount *account,
                                             const char *group_id);
void signal_group_sync_update_title(PurpleChat *chat, const char *group_id,
                                    const char *title);
gboolean signal_group_sync_defer_join(GHashTable *pending,
                                      gboolean snapshot_complete,
                                      const char *group_id);
GPtrArray *signal_group_sync_take_active_joins(GHashTable *pending,
                                               GHashTable *active);

#endif
