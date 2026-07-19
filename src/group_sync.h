/* SPDX-License-Identifier: GPL-3.0-or-later */
#ifndef SIGNAL_GROUP_SYNC_H
#define SIGNAL_GROUP_SYNC_H

#include <purple.h>

#define SIGNAL_SYNCED_GROUP_KEY "signal-purple-synced-group"
#define SIGNAL_GROUP_COMPONENT_KEY "group-id"
#define SIGNAL_LEGACY_GROUP_COMPONENT_KEY "group-key"

PurpleChat *signal_group_sync_find_chat(PurpleAccount *account,
                                        const char *group_id,
                                        guint *removed);

#endif
