/* SPDX-License-Identifier: GPL-3.0-or-later */
#ifndef SIGNAL_BLIST_SYNC_H
#define SIGNAL_BLIST_SYNC_H

#include <purple.h>

#define SIGNAL_SYNCED_BUDDY_KEY "signal-purple-synced-contact"
#define SIGNAL_SYNCED_GROUP_KEY "signal-purple-synced-group"

#define SIGNAL_LEGACY_BUDDY_GROUP "Signal"
#define SIGNAL_LEGACY_CHAT_GROUP "Signal groups"

void signal_blist_sync_add_buddy(PurpleBuddy *buddy);
PurpleBuddy *signal_blist_sync_find_buddy(PurpleAccount *account,
                                          const char *name);
PurpleBuddy *signal_blist_sync_migrate_buddy(PurpleBuddy *buddy);
gboolean signal_blist_sync_is_legacy_buddy_group(PurpleGroup *group);
void signal_blist_sync_remove_empty_legacy_buddy_group(void);

void signal_blist_sync_add_chat(PurpleChat *chat);
void signal_blist_sync_migrate_chat(PurpleChat *chat);
gboolean signal_blist_sync_is_legacy_chat_group(PurpleGroup *group);
void signal_blist_sync_remove_empty_legacy_chat_group(void);

#endif
