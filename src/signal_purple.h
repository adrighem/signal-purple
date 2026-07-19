/* SPDX-License-Identifier: GPL-3.0-or-later */
#ifndef SIGNAL_PURPLE_H
#define SIGNAL_PURPLE_H

#include <glib.h>
#include <purple.h>

#include "contact_sync.h"
#include "group_sync.h"
#include "signal_core.h"

#define SIGNAL_PLUGIN_ID "prpl-adrighem-signal"
#define SIGNAL_MAX_MESSAGE_BYTES (64u * 1024u)

typedef struct {
    PurpleConnection *gc;
    SignalCore *core;
    GSource *poll_source;
    GHashTable *group_ids_by_key;
    GHashTable *group_keys_by_id;
    GHashTable *group_titles_by_key;
    GHashTable *group_members_by_key;
    SignalContactSync contact_sync;
    SignalContactSync group_sync;
    GBytes *link_qr;
    char *store_path;
    guint next_group_id;
    guint contact_sync_contacts;
    guint contact_sync_created;
    guint contact_sync_removed;
    guint group_sync_groups;
    guint group_sync_created;
    guint group_sync_removed;
    guint64 next_request_id;
    gboolean closing;
} SignalConnection;

void signal_login(PurpleAccount *account);
void signal_close(PurpleConnection *gc);
int signal_send_im(PurpleConnection *gc, const char *who, const char *message,
                   PurpleMessageFlags flags);
unsigned int signal_send_typing(PurpleConnection *gc, const char *who,
                                PurpleTypingState state);

GList *signal_chat_info(PurpleConnection *gc);
GHashTable *signal_chat_info_defaults(PurpleConnection *gc,
                                      const char *chat_name);
void signal_join_chat(PurpleConnection *gc, GHashTable *components);
char *signal_get_chat_name(GHashTable *components);

void signal_chat_leave(PurpleConnection *gc, int id);
int signal_chat_send(PurpleConnection *gc, int id, const char *message,
                     PurpleMessageFlags flags);

char *signal_plaintext_from_markup(const char *markup);
PurpleMessageFlags signal_message_flags(gboolean outgoing);
char *signal_store_path(PurpleAccount *account, GError **error);
char *signal_secret_get_or_create(PurpleAccount *account,
                                  const char *store_path,
                                  GError **error);

#endif
