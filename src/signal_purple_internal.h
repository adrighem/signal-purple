/* SPDX-License-Identifier: GPL-3.0-or-later */
#ifndef SIGNAL_PURPLE_INTERNAL_H
#define SIGNAL_PURPLE_INTERNAL_H

#include "signal_purple.h"

#define SIGNAL_MAX_PENDING_READS 4096u
#define SIGNAL_PENDING_READ_RETRY_MILLISECONDS 100u

typedef struct {
    char *peer_id;
    PurpleConvChatBuddyFlags flags;
} SignalGroupMember;

typedef struct {
    char *peer_id;
    char *chat_id;
    guint64 timestamp;
    gboolean eligible;
} SignalPendingRead;

typedef struct {
    GBytes *bytes;
    gsize size;
} SignalAttachment;

typedef struct {
    SignalConnection *connection;
    char *group_key;
    char *title;
} SignalGroupLeaveRequest;

SignalConnection *signal_connection_data(PurpleConnection *gc);
gboolean signal_group_is_active(SignalConnection *connection,
                                const char *group_key);
gboolean signal_group_can_send(SignalConnection *connection,
                               const char *group_key);
const char *signal_group_title(SignalConnection *connection,
                               const char *group_key);
PurpleConversation *signal_open_group(SignalConnection *connection,
                                      const char *group_key,
                                      const char *title);
gboolean signal_event_can_present(SignalConnection *connection,
                                  PurpleConversation *conversation,
                                  const SignalEvent *event);
void signal_queue_read(SignalConnection *connection,
                       PurpleConversation *conversation,
                       const SignalEvent *event);
void signal_queue_final_read(SignalConnection *connection,
                             PurpleConversation *conversation,
                             const SignalEvent *event);
void signal_flush_pending_reads(SignalConnection *connection);

PurpleXfer *signal_new_attachment_xfer(SignalConnection *connection,
                                       const char *display_peer,
                                       const char *recipient,
                                       gboolean group);
void signal_outgoing_attachment_detach(SignalOutgoingAttachment *attachment);
gboolean signal_outgoing_attachment_complete(SignalConnection *connection,
                                             const SignalEvent *event);
gboolean signal_outgoing_attachment_failed(SignalConnection *connection,
                                           const SignalEvent *event);
gboolean signal_deliver_attachment(SignalConnection *connection,
                                   const SignalEvent *event);

#endif
