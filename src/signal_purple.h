/* SPDX-License-Identifier: GPL-3.0-or-later */
#ifndef SIGNAL_PURPLE_H
#define SIGNAL_PURPLE_H

#include <glib.h>
#include <purple.h>

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
    GBytes *link_qr;
    char *store_path;
    guint next_group_id;
    guint64 next_request_id;
    gboolean closing;
} SignalConnection;

void signal_login(PurpleAccount *account);
void signal_close(PurpleConnection *gc);
int signal_send_im(PurpleConnection *gc, const char *who, const char *message,
                   PurpleMessageFlags flags);
unsigned int signal_send_typing(PurpleConnection *gc, const char *who,
                                PurpleTypingState state);

void signal_chat_leave(PurpleConnection *gc, int id);
int signal_chat_send(PurpleConnection *gc, int id, const char *message,
                     PurpleMessageFlags flags);

char *signal_plaintext_from_markup(const char *markup);
char *signal_store_path(PurpleAccount *account, GError **error);
char *signal_secret_get_or_create(PurpleAccount *account,
                                  const char *store_path,
                                  GError **error);

#endif
