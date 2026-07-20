/* SPDX-License-Identifier: GPL-3.0-or-later */
#include "signal_purple.h"

#include <errno.h>
#include <glib-unix.h>
#include <gio/gio.h>
#include <libsecret/secret.h>

#define SIGNAL_MAX_PENDING_ATTACHMENT_BYTES (64u * 1024u * 1024u)

static gsize signal_pending_attachment_bytes;

typedef struct {
    char *peer_id;
    PurpleConvChatBuddyFlags flags;
} SignalGroupMember;

typedef struct {
    char *peer_id;
    char *chat_id;
    guint64 timestamp;
} SignalPendingRead;

typedef struct {
    GBytes *bytes;
    gsize size;
} SignalAttachment;

typedef struct {
    SignalConnection *connection;
    char *recipient;
    guint64 request_id;
    gboolean group;
} SignalOutgoingAttachment;

typedef struct {
    SignalConnection *connection;
    char *group_key;
    char *title;
} SignalGroupLeaveRequest;

static gboolean signal_group_is_active(SignalConnection *connection,
                                       const char *group_key);
static gboolean signal_group_can_send(SignalConnection *connection,
                                      const char *group_key);

static void
signal_outgoing_attachment_free(PurpleXfer *xfer)
{
    SignalOutgoingAttachment *attachment;

    if (xfer == NULL || xfer->data == NULL)
        return;
    attachment = xfer->data;
    xfer->data = NULL;
    if (attachment->connection != NULL && attachment->request_id != 0)
        g_hash_table_remove(attachment->connection->outgoing_attachments,
                            &attachment->request_id);
    g_free(attachment->recipient);
    g_free(attachment);
}

static void
signal_outgoing_attachment_cancel(PurpleXfer *xfer)
{
    SignalOutgoingAttachment *attachment = xfer->data;

    if (attachment != NULL && attachment->connection != NULL &&
        !attachment->connection->closing && attachment->request_id != 0)
        signal_core_cancel_attachment(attachment->connection->core,
                                      attachment->request_id);
    signal_outgoing_attachment_free(xfer);
}

static void
signal_outgoing_attachment_init(PurpleXfer *xfer)
{
    SignalOutgoingAttachment *attachment = xfer->data;
    g_autofree char *contents = NULL;
    g_autofree char *filename = NULL;
    g_autofree char *content_type = NULL;
    g_autofree char *mime_type = NULL;
    g_autoptr(GError) error = NULL;
    gsize size = 0;
    gboolean uncertain = FALSE;
    SignalStatus status;
    guint64 *key;

    if (attachment == NULL || attachment->connection == NULL ||
        attachment->connection->closing)
        return;
    if (attachment->group &&
        !signal_group_can_send(attachment->connection,
                               attachment->recipient)) {
        purple_xfer_error(PURPLE_XFER_SEND, purple_xfer_get_account(xfer),
                          purple_xfer_get_remote_user(xfer),
                          "This Signal group is no longer active");
        purple_xfer_cancel_local(xfer);
        return;
    }
    if (!g_file_get_contents(purple_xfer_get_local_filename(xfer), &contents,
                             &size, &error)) {
        purple_xfer_error(PURPLE_XFER_SEND, purple_xfer_get_account(xfer),
                          purple_xfer_get_remote_user(xfer), error->message);
        purple_xfer_cancel_local(xfer);
        return;
    }
    if (size == 0 || size > SIGNAL_MAX_ATTACHMENT_BYTES) {
        purple_xfer_error(PURPLE_XFER_SEND, purple_xfer_get_account(xfer),
                          purple_xfer_get_remote_user(xfer),
                          "Signal attachments must be between 1 byte and 25 MiB");
        purple_xfer_cancel_local(xfer);
        return;
    }

    filename = g_path_get_basename(purple_xfer_get_local_filename(xfer));
    content_type = g_content_type_guess(filename, (const guchar *)contents,
                                        MIN(size, (gsize)512), &uncertain);
    (void)uncertain;
    if (content_type != NULL)
        mime_type = g_content_type_get_mime_type(content_type);
    if (mime_type == NULL)
        mime_type = g_strdup("application/octet-stream");

    attachment->request_id = attachment->connection->next_request_id++;
    purple_xfer_start(xfer, -1, NULL, 0);
    if (xfer->data == NULL)
        return;
    if (attachment->group) {
        status = signal_core_send_group_attachment(
            attachment->connection->core, attachment->request_id,
            attachment->recipient, filename, mime_type,
            (const guint8 *)contents, size);
    } else {
        status = signal_core_send_attachment(
            attachment->connection->core, attachment->request_id,
            attachment->recipient, filename, mime_type,
            (const guint8 *)contents, size);
    }
    if (status != SIGNAL_STATUS_OK) {
        attachment->request_id = 0;
        purple_xfer_error(PURPLE_XFER_SEND, purple_xfer_get_account(xfer),
                          purple_xfer_get_remote_user(xfer),
                          "The Signal attachment could not be queued");
        purple_xfer_cancel_local(xfer);
        return;
    }

    key = g_new(guint64, 1);
    *key = attachment->request_id;
    purple_xfer_ref(xfer);
    g_hash_table_insert(attachment->connection->outgoing_attachments, key,
                        xfer);
}

static gboolean
signal_outgoing_attachment_complete(SignalConnection *connection,
                                    const SignalEvent *event)
{
    PurpleXfer *xfer;

    xfer = event->request_id != 0
               ? g_hash_table_lookup(connection->outgoing_attachments,
                                     &event->request_id)
               : NULL;
    if (xfer == NULL)
        return FALSE;
    purple_xfer_set_bytes_sent(xfer, purple_xfer_get_size(xfer));
    purple_xfer_update_progress(xfer);
    purple_xfer_set_completed(xfer, TRUE);
    purple_xfer_end(xfer);
    return TRUE;
}

static gboolean
signal_outgoing_attachment_failed(SignalConnection *connection,
                                  const SignalEvent *event)
{
    PurpleXfer *xfer;

    xfer = event->request_id != 0
               ? g_hash_table_lookup(connection->outgoing_attachments,
                                     &event->request_id)
               : NULL;
    if (xfer == NULL)
        return FALSE;
    purple_xfer_error(PURPLE_XFER_SEND, purple_xfer_get_account(xfer),
                      purple_xfer_get_remote_user(xfer),
                      event->text != NULL ? event->text
                                          : "Signal attachment send failed");
    purple_xfer_cancel_remote(xfer);
    return TRUE;
}

static void
signal_attachment_free(PurpleXfer *xfer)
{
    SignalAttachment *attachment;

    if (xfer == NULL || xfer->data == NULL)
        return;
    attachment = xfer->data;
    xfer->data = NULL;
    g_assert(signal_pending_attachment_bytes >= attachment->size);
    signal_pending_attachment_bytes -= attachment->size;
    g_clear_pointer(&attachment->bytes, g_bytes_unref);
    g_free(attachment);
}

static void
signal_attachment_cancel(PurpleXfer *xfer)
{
    signal_attachment_free(xfer);
}

static void
signal_attachment_start(PurpleXfer *xfer)
{
    SignalAttachment *attachment = xfer->data;
    gconstpointer bytes;
    gsize size;

    if (attachment == NULL || attachment->bytes == NULL) {
        purple_xfer_cancel_local(xfer);
        return;
    }
    bytes = g_bytes_get_data(attachment->bytes, &size);
    if (!purple_xfer_write_file(xfer, bytes, size))
        return;

    purple_xfer_update_progress(xfer);
    purple_xfer_set_completed(xfer, TRUE);
    purple_xfer_end(xfer);
}

static void
signal_attachment_init(PurpleXfer *xfer)
{
    purple_xfer_start(xfer, -1, NULL, 0);
}

static void
signal_deliver_attachment(SignalConnection *connection,
                          const SignalEvent *event)
{
    PurpleAccount *account;
    PurpleXfer *xfer;
    SignalAttachment *attachment;
    g_autofree char *filename = NULL;
    const char *peer;

    if (event->data == NULL || event->data_len == 0)
        return;
    if (event->data_len > SIGNAL_MAX_ATTACHMENT_BYTES ||
        signal_pending_attachment_bytes >
            SIGNAL_MAX_PENDING_ATTACHMENT_BYTES - event->data_len) {
        purple_notify_error(
            connection, "Signal attachment rejected",
            "Too much attachment data is waiting for a save location",
            "Save or reject pending transfers, then ask the sender to resend the attachment.");
        return;
    }
    peer = event->peer_id != NULL && event->peer_id[0] != '\0'
               ? event->peer_id
               : "Signal contact";
    filename = g_path_get_basename(
        event->title != NULL && event->title[0] != '\0'
            ? event->title
            : "signal-attachment");
    if (g_str_equal(filename, ".") || g_str_equal(filename, "..") ||
        g_str_equal(filename, G_DIR_SEPARATOR_S)) {
        g_free(g_steal_pointer(&filename));
        filename = g_strdup("signal-attachment");
    }

    account = purple_connection_get_account(connection->gc);
    xfer = purple_xfer_new(account, PURPLE_XFER_RECEIVE, peer);
    if (xfer == NULL)
        return;
    attachment = g_new0(SignalAttachment, 1);
    attachment->bytes = g_bytes_new(event->data, event->data_len);
    attachment->size = event->data_len;
    signal_pending_attachment_bytes += attachment->size;
    xfer->data = attachment;
    purple_xfer_set_filename(xfer, filename);
    purple_xfer_set_size(xfer, event->data_len);
    if (event->text != NULL && event->text[0] != '\0')
        purple_xfer_set_message(xfer, event->text);
    purple_xfer_set_init_fnc(xfer, signal_attachment_init);
    purple_xfer_set_start_fnc(xfer, signal_attachment_start);
    purple_xfer_set_end_fnc(xfer, signal_attachment_free);
    purple_xfer_set_request_denied_fnc(xfer, signal_attachment_free);
    purple_xfer_set_cancel_recv_fnc(xfer, signal_attachment_cancel);
    purple_xfer_request(xfer);
}

static void
signal_pending_read_free(gpointer data)
{
    SignalPendingRead *read = data;

    if (read == NULL)
        return;
    g_free(read->peer_id);
    g_free(read->chat_id);
    g_free(read);
}

static gboolean
signal_send_read(SignalConnection *connection, const SignalPendingRead *read)
{
    return signal_core_mark_read(
               connection->core, connection->next_request_id++,
               read->peer_id, read->timestamp) == SIGNAL_STATUS_OK;
}

static void
signal_queue_read(SignalConnection *connection, PurpleConversation *conversation,
                  const SignalEvent *event)
{
    SignalPendingRead *read;

    if ((event->flags & SIGNAL_EVENT_FLAG_OUTGOING) != 0 ||
        event->peer_id == NULL || event->timestamp_ms == 0)
        return;
    read = g_new0(SignalPendingRead, 1);
    read->peer_id = g_strdup(event->peer_id);
    read->chat_id = g_strdup(event->chat_id);
    read->timestamp = event->timestamp_ms;
    if (purple_conversation_has_focus(conversation) &&
        signal_send_read(connection, read)) {
        signal_pending_read_free(read);
        return;
    }
    g_ptr_array_add(connection->pending_reads, read);
}

static void
signal_group_member_free(gpointer data)
{
    SignalGroupMember *member = data;

    if (member == NULL)
        return;
    g_free(member->peer_id);
    g_free(member);
}

static GPtrArray *
signal_group_members(SignalConnection *connection, const char *group_key,
                     gboolean create)
{
    GPtrArray *members = g_hash_table_lookup(connection->group_members_by_key,
                                             group_key);

    if (members == NULL && create) {
        members = g_ptr_array_new_with_free_func(signal_group_member_free);
        g_hash_table_insert(connection->group_members_by_key,
                            g_strdup(group_key), members);
    }
    return members;
}

static guint
signal_group_id(SignalConnection *connection, const char *group_key,
                const char *title)
{
    gpointer existing;
    guint id;

    if (g_hash_table_lookup_extended(connection->group_ids_by_key, group_key,
                                     NULL, &existing)) {
        id = GPOINTER_TO_UINT(existing);
    } else {
        id = connection->next_group_id++;
        g_hash_table_insert(connection->group_ids_by_key, g_strdup(group_key),
                            GUINT_TO_POINTER(id));
        g_hash_table_insert(connection->group_keys_by_id, GUINT_TO_POINTER(id),
                            g_strdup(group_key));
    }

    if (title != NULL && title[0] != '\0')
        g_hash_table_replace(connection->group_titles_by_key,
                             g_strdup(group_key), g_strdup(title));

    return id;
}

static const char *
signal_group_title(SignalConnection *connection, const char *group_key)
{
    const char *title = g_hash_table_lookup(connection->group_titles_by_key,
                                            group_key);
    return title != NULL && title[0] != '\0' ? title : "Signal group";
}

static gboolean
signal_group_is_active(SignalConnection *connection, const char *group_key)
{
    return connection != NULL && group_key != NULL &&
           g_hash_table_contains(connection->active_group_keys, group_key);
}

static gboolean
signal_group_can_send(SignalConnection *connection, const char *group_key)
{
    return signal_group_is_active(connection, group_key) &&
           !g_hash_table_contains(connection->pending_group_leaves,
                                  group_key);
}

static const char *
signal_group_display_title(SignalConnection *connection, const char *group_key)
{
    PurpleAccount *account = purple_connection_get_account(connection->gc);
    PurpleChat *chat = signal_group_sync_lookup_chat(account, group_key);
    const char *title = chat != NULL ? purple_chat_get_name(chat) : NULL;

    if (title != NULL && title[0] != '\0' &&
        g_strcmp0(title, group_key) != 0)
        return title;
    return signal_group_title(connection, group_key);
}

static void
signal_update_open_group_title(SignalConnection *connection,
                               const char *group_key)
{
    gpointer raw_id;
    PurpleConversation *conversation;

    if (!g_hash_table_lookup_extended(connection->group_ids_by_key, group_key,
                                      NULL, &raw_id))
        return;
    conversation = purple_find_chat(connection->gc,
                                    (int)GPOINTER_TO_UINT(raw_id));
    if (conversation != NULL)
        purple_conversation_set_title(
            conversation, signal_group_display_title(connection, group_key));
}

static void
signal_remove_group_pending_reads(SignalConnection *connection,
                                  const char *group_key)
{
    for (guint index = connection->pending_reads->len; index > 0; index--) {
        SignalPendingRead *read = g_ptr_array_index(connection->pending_reads,
                                                    index - 1);

        if (g_strcmp0(read->chat_id, group_key) == 0)
            g_ptr_array_remove_index(connection->pending_reads, index - 1);
    }
}

static void
signal_deactivate_group(SignalConnection *connection, const char *group_key,
                        gboolean remove_saved_chat)
{
    PurpleAccount *account;
    gpointer raw_id = NULL;

    if (connection == NULL || group_key == NULL || group_key[0] == '\0')
        return;

    account = purple_connection_get_account(connection->gc);
    if (g_hash_table_lookup_extended(connection->group_ids_by_key, group_key,
                                     NULL, &raw_id)) {
        guint id = GPOINTER_TO_UINT(raw_id);

        if (purple_find_chat(connection->gc, (int)id) != NULL)
            serv_got_chat_left(connection->gc, (int)id);
        g_hash_table_remove(connection->group_keys_by_id, raw_id);
        g_hash_table_remove(connection->group_ids_by_key, group_key);
    }

    if (remove_saved_chat)
        signal_group_sync_remove_managed_chats(account, group_key);
    g_hash_table_remove(connection->active_group_keys, group_key);
    g_hash_table_remove(connection->pending_group_leaves, group_key);
    g_hash_table_remove(connection->group_titles_by_key, group_key);
    g_hash_table_remove(connection->group_members_by_key, group_key);
    signal_remove_group_pending_reads(connection, group_key);
}

static void
signal_group_leave_request_free(gpointer data)
{
    SignalGroupLeaveRequest *request = data;

    if (request == NULL)
        return;
    g_free(request->group_key);
    g_free(request->title);
    g_free(request);
}

static void
signal_finish_group_leave_request(SignalGroupLeaveRequest *request)
{
    SignalConnection *connection;

    if (request == NULL)
        return;
    connection = request->connection;
    if (connection != NULL && connection->group_leave_requests != NULL &&
        g_ptr_array_remove(connection->group_leave_requests, request))
        return;
    signal_group_leave_request_free(request);
}

static gboolean
signal_group_leave_prompted(SignalConnection *connection,
                            const char *group_key)
{
    for (guint index = 0; index < connection->group_leave_requests->len;
         index++) {
        SignalGroupLeaveRequest *request = g_ptr_array_index(
            connection->group_leave_requests, index);

        if (g_strcmp0(request->group_key, group_key) == 0)
            return TRUE;
    }
    return FALSE;
}

static void
signal_link_keep_waiting(void *user_data, PurpleRequestFields *fields)
{
    (void)user_data;
    (void)fields;
}

static void
signal_link_cancel(void *user_data, PurpleRequestFields *fields)
{
    SignalConnection *connection = user_data;

    (void)fields;
    if (connection == NULL || connection->closing)
        return;

    purple_account_disconnect(purple_connection_get_account(connection->gc));
}

static void
signal_show_link_qr(SignalConnection *connection, const SignalEvent *event)
{
    PurpleRequestFields *fields;
    PurpleRequestFieldGroup *group;
    PurpleRequestField *field;
    gconstpointer bytes;
    gsize size;

    purple_request_close_with_handle(&connection->link_qr);
    g_clear_pointer(&connection->link_qr, g_bytes_unref);
    connection->link_qr = g_bytes_new(event->data, event->data_len);
    bytes = g_bytes_get_data(connection->link_qr, &size);

    fields = purple_request_fields_new();
    group = purple_request_field_group_new(NULL);
    purple_request_fields_add_group(fields, group);

    field = purple_request_field_image_new("qr", "Linking QR code",
                                           bytes, size);
    purple_request_field_group_add_field(group, field);

    field = purple_request_field_string_new(
        "uri", "Provisioning URI (for clients without image support)",
        event->text != NULL ? event->text : "", FALSE);
    purple_request_field_string_set_editable(field, FALSE);
    purple_request_field_group_add_field(group, field);

    purple_request_fields(
        &connection->link_qr, "Link signal-purple", "Scan this QR code with Signal",
        "On your phone, open Signal Settings > Linked devices > Link new device.",
        fields, "I've scanned it", G_CALLBACK(signal_link_keep_waiting),
        "Cancel linking", G_CALLBACK(signal_link_cancel),
        purple_connection_get_account(connection->gc), NULL, NULL,
        connection);
}

static void
signal_begin_contact_sync(SignalConnection *connection)
{
    signal_contact_sync_begin(&connection->contact_sync);
    connection->contact_sync_contacts = 0;
    connection->contact_sync_created = 0;
    connection->contact_sync_removed = 0;
}

static void
signal_add_contact(SignalConnection *connection, const SignalEvent *event)
{
    PurpleAccount *account;
    PurpleBuddy *buddy;
    gboolean managed;
    const char *alias;

    if (event->peer_id == NULL || event->peer_id[0] == '\0')
        return;

    connection->contact_sync_contacts++;
    signal_contact_sync_mark(&connection->contact_sync, event->peer_id);
    account = purple_connection_get_account(connection->gc);
    buddy = signal_blist_sync_find_buddy(account, event->peer_id);
    if (buddy != NULL)
        buddy = signal_blist_sync_migrate_buddy(buddy);
    managed =
        buddy != NULL &&
        purple_blist_node_get_bool(PURPLE_BLIST_NODE(buddy),
                                   SIGNAL_SYNCED_BUDDY_KEY);
    if (buddy == NULL) {
        buddy = purple_buddy_new(account, event->peer_id, NULL);
        signal_blist_sync_add_buddy(buddy);
        managed = TRUE;
        connection->contact_sync_created++;
    }

    if (managed)
        purple_blist_node_set_bool(PURPLE_BLIST_NODE(buddy),
                                   SIGNAL_SYNCED_BUDDY_KEY, TRUE);
    alias = event->title != NULL && event->title[0] != '\0'
                ? event->title
                : event->text;
    purple_blist_server_alias_buddy(
        buddy, alias != NULL && alias[0] != '\0' ? alias : NULL);
    /* Signal has no contact presence. Treat synchronized contacts as reachable
     * while the linked account is connected so Purple does not hide the whole
     * address book behind its offline-buddy filter. */
    purple_prpl_got_user_status(account, event->peer_id, "available", NULL);
}

static void
signal_end_contact_sync(SignalConnection *connection)
{
    PurpleAccount *account;
    GSList *buddies;

    if (!connection->contact_sync.active)
        return;

    account = purple_connection_get_account(connection->gc);
    buddies = purple_find_buddies(account, NULL);
    for (GSList *item = buddies; item != NULL; item = item->next) {
        PurpleBuddy *buddy = item->data;
        PurpleBlistNode *node = PURPLE_BLIST_NODE(buddy);

        if (purple_blist_node_get_bool(node, SIGNAL_SYNCED_BUDDY_KEY) &&
            signal_contact_sync_should_remove(
                &connection->contact_sync, purple_buddy_get_name(buddy))) {
            gboolean was_legacy = signal_blist_sync_is_legacy_buddy_group(
                purple_buddy_get_group(buddy));

            purple_blist_remove_buddy(buddy);
            if (was_legacy)
                signal_blist_sync_remove_empty_legacy_buddy_group();
            connection->contact_sync_removed++;
        }
    }
    g_slist_free(buddies);
    signal_contact_sync_end(&connection->contact_sync);
    purple_debug_info(
        "signal-purple",
        "Applied contact snapshot: %u contacts, %u created, %u removed\n",
        connection->contact_sync_contacts, connection->contact_sync_created,
        connection->contact_sync_removed);
}

static void
signal_begin_group_sync(SignalConnection *connection)
{
    signal_contact_sync_begin(&connection->group_sync);
    g_hash_table_remove_all(connection->group_members_by_key);
    connection->group_sync_groups = 0;
    connection->group_sync_created = 0;
    connection->group_sync_removed = 0;
}

static void
signal_add_group(SignalConnection *connection, const SignalEvent *event)
{
    PurpleAccount *account;
    PurpleChat *chat;
    GHashTable *components;
    gboolean managed;
    const char *title;

    if (event->chat_id == NULL || event->chat_id[0] == '\0')
        return;

    signal_group_id(connection, event->chat_id, event->title);
    signal_contact_sync_mark(&connection->group_sync, event->chat_id);
    connection->group_sync_groups++;
    account = purple_connection_get_account(connection->gc);
    chat = signal_group_sync_find_chat(account, event->chat_id,
                                       &connection->group_sync_removed);
    if (chat != NULL)
        signal_blist_sync_migrate_chat(chat);
    managed = chat != NULL &&
              purple_blist_node_get_bool(PURPLE_BLIST_NODE(chat),
                                         SIGNAL_SYNCED_GROUP_KEY);
    title = signal_group_title(connection, event->chat_id);
    signal_group_members(connection, event->chat_id, TRUE);

    if (chat == NULL) {
        components = g_hash_table_new_full(g_str_hash, g_str_equal, g_free,
                                           g_free);
        g_hash_table_insert(components, g_strdup(SIGNAL_GROUP_COMPONENT_KEY),
                            g_strdup(event->chat_id));
        chat = purple_chat_new(account, title, components);
        signal_blist_sync_add_chat(chat);
        managed = TRUE;
        connection->group_sync_created++;
    }

    if (managed) {
        purple_blist_node_set_bool(PURPLE_BLIST_NODE(chat),
                                   SIGNAL_SYNCED_GROUP_KEY, TRUE);
        signal_group_sync_update_title(chat, event->chat_id, title);
    }
    signal_update_open_group_title(connection, event->chat_id);
}

static void
signal_add_group_member(SignalConnection *connection, const SignalEvent *event)
{
    GPtrArray *members;
    SignalGroupMember *member;

    if (event->chat_id == NULL || event->chat_id[0] == '\0' ||
        event->peer_id == NULL || event->peer_id[0] == '\0')
        return;

    members = signal_group_members(connection, event->chat_id, TRUE);
    member = g_new0(SignalGroupMember, 1);
    member->peer_id = g_strdup(event->peer_id);
    member->flags = event->value != 0 ? PURPLE_CBFLAGS_OP
                                     : PURPLE_CBFLAGS_NONE;
    g_ptr_array_add(members, member);
}

static void
signal_refresh_group_members(SignalConnection *connection,
                             const char *group_key)
{
    gpointer raw_id;
    PurpleConversation *conversation;
    GPtrArray *members;
    GList *users = NULL;
    GList *flags = NULL;

    if (!g_hash_table_lookup_extended(connection->group_ids_by_key, group_key,
                                      NULL, &raw_id))
        return;
    conversation = purple_find_chat(connection->gc,
                                    (int)GPOINTER_TO_UINT(raw_id));
    if (conversation == NULL)
        return;

    purple_conv_chat_clear_users(PURPLE_CONV_CHAT(conversation));
    members = signal_group_members(connection, group_key, FALSE);
    if (members == NULL)
        return;

    for (guint index = 0; index < members->len; index++) {
        SignalGroupMember *member = g_ptr_array_index(members, index);
        users = g_list_prepend(users, member->peer_id);
        flags = g_list_prepend(flags, GINT_TO_POINTER(member->flags));
    }
    users = g_list_reverse(users);
    flags = g_list_reverse(flags);
    if (users != NULL)
        purple_conv_chat_add_users(PURPLE_CONV_CHAT(conversation), users,
                                   NULL, flags, FALSE);
    g_list_free(users);
    g_list_free(flags);
}

static void
signal_end_group_sync(SignalConnection *connection)
{
    PurpleAccount *account;
    PurpleBlistNode *node;
    GHashTableIter active_iter;
    GHashTableIter seen_iter;
    GHashTableIter member_iter;
    gpointer active_group_key;
    gpointer seen_group_key;
    gpointer member_group_key;
    g_autoptr(GPtrArray) stale_groups =
        g_ptr_array_new_with_free_func(g_free);

    if (!connection->group_sync.active)
        return;

    g_hash_table_iter_init(&active_iter, connection->active_group_keys);
    while (g_hash_table_iter_next(&active_iter, &active_group_key, NULL)) {
        if (signal_contact_sync_should_remove(&connection->group_sync,
                                              active_group_key))
            g_ptr_array_add(stale_groups, g_strdup(active_group_key));
    }
    for (guint index = 0; index < stale_groups->len; index++)
        signal_deactivate_group(connection,
                                g_ptr_array_index(stale_groups, index), FALSE);

    g_hash_table_remove_all(connection->active_group_keys);
    g_hash_table_iter_init(&seen_iter, connection->group_sync.seen);
    while (g_hash_table_iter_next(&seen_iter, &seen_group_key, NULL))
        g_hash_table_add(connection->active_group_keys,
                         g_strdup(seen_group_key));

    account = purple_connection_get_account(connection->gc);
    node = purple_blist_get_root();
    while (node != NULL) {
        PurpleBlistNode *next = purple_blist_node_next(node, FALSE);

        if (PURPLE_BLIST_NODE_IS_CHAT(node)) {
            PurpleChat *chat = PURPLE_CHAT(node);
            GHashTable *components = purple_chat_get_components(chat);
            const char *group_key = g_hash_table_lookup(
                components, SIGNAL_GROUP_COMPONENT_KEY);

            if (group_key == NULL)
                group_key = g_hash_table_lookup(
                    components, SIGNAL_LEGACY_GROUP_COMPONENT_KEY);

            if (purple_chat_get_account(chat) == account &&
                purple_blist_node_get_bool(node, SIGNAL_SYNCED_GROUP_KEY) &&
                signal_contact_sync_should_remove(&connection->group_sync,
                                                  group_key)) {
                gboolean was_legacy = signal_blist_sync_is_legacy_chat_group(
                    purple_chat_get_group(chat));

                purple_blist_remove_chat(chat);
                if (was_legacy)
                    signal_blist_sync_remove_empty_legacy_chat_group();
                connection->group_sync_removed++;
            }
        }
        node = next;
    }

    signal_contact_sync_end(&connection->group_sync);
    g_hash_table_iter_init(&member_iter, connection->group_members_by_key);
    while (g_hash_table_iter_next(&member_iter, &member_group_key, NULL))
        signal_refresh_group_members(connection, member_group_key);
    purple_debug_info(
        "signal-purple",
        "Applied group snapshot: %u groups, %u created, %u removed\n",
        connection->group_sync_groups, connection->group_sync_created,
        connection->group_sync_removed);
}

static PurpleConversation *
signal_open_group(SignalConnection *connection, const char *group_key,
                  const char *title)
{
    PurpleConversation *conversation;
    guint id = signal_group_id(connection, group_key, title);

    conversation = purple_find_chat(connection->gc, (int)id);
    /* Purple uses the conversation name to match saved chats. Keep that
     * identity stable and put the mutable, human-readable name in the title. */
    if (conversation == NULL)
        conversation = serv_got_joined_chat(connection->gc, (int)id,
                                             group_key);
    purple_conversation_set_title(
        conversation, signal_group_display_title(connection, group_key));
    purple_conversation_set_logging(conversation, FALSE);
    signal_refresh_group_members(connection, group_key);
    return conversation;
}

static void
signal_deliver_direct(SignalConnection *connection, const SignalEvent *event)
{
    PurpleAccount *account;
    PurpleConversation *conversation;
    g_autofree char *escaped = NULL;
    time_t timestamp;

    if (event->peer_id == NULL || event->text == NULL)
        return;

    account = purple_connection_get_account(connection->gc);
    conversation = purple_find_conversation_with_account(
        PURPLE_CONV_TYPE_IM, event->peer_id, account);
    if (conversation == NULL)
        conversation = purple_conversation_new(PURPLE_CONV_TYPE_IM, account,
                                               event->peer_id);
    purple_conversation_set_logging(conversation, FALSE);

    escaped = g_markup_escape_text(event->text, -1);
    timestamp = event->timestamp_ms > 0
                    ? (time_t)(event->timestamp_ms / 1000)
                    : time(NULL);

    if ((event->flags & SIGNAL_EVENT_FLAG_OUTGOING) == 0) {
        serv_got_im(connection->gc, event->peer_id, escaped,
                    signal_message_flags(FALSE), timestamp);
        signal_queue_read(connection, conversation, event);
        return;
    }

    purple_conv_im_write(PURPLE_CONV_IM(conversation),
                         purple_account_get_username(account), escaped,
                         signal_message_flags(TRUE),
                         timestamp);
}

static void
signal_deliver_group(SignalConnection *connection, const SignalEvent *event)
{
    guint id;
    g_autofree char *escaped = NULL;
    time_t timestamp;

    if (event->chat_id == NULL || event->peer_id == NULL || event->text == NULL)
        return;

    /* The backend emits group content only after confirming local membership.
     * This keeps a newly joined group usable before the next full snapshot. */
    g_hash_table_add(connection->active_group_keys,
                     g_strdup(event->chat_id));
    PurpleConversation *conversation = signal_open_group(
        connection, event->chat_id, event->title);
    id = signal_group_id(connection, event->chat_id, event->title);
    escaped = g_markup_escape_text(event->text, -1);
    timestamp = event->timestamp_ms > 0
                    ? (time_t)(event->timestamp_ms / 1000)
                    : time(NULL);
    serv_got_chat_in(connection->gc, (int)id, event->peer_id,
                     signal_message_flags(
                         (event->flags & SIGNAL_EVENT_FLAG_OUTGOING) != 0),
                     escaped, timestamp);
    signal_queue_read(connection, conversation, event);
}

static void
signal_conversation_updated(PurpleConversation *conversation,
                            PurpleConvUpdateType update, gpointer user_data)
{
    SignalConnection *connection = user_data;
    const char *direct_peer = NULL;
    const char *group_key = NULL;

    (void)update;
    if (connection->closing || !purple_conversation_has_focus(conversation) ||
        purple_conversation_get_account(conversation) !=
            purple_connection_get_account(connection->gc))
        return;
    if (purple_conversation_get_type(conversation) == PURPLE_CONV_TYPE_IM) {
        direct_peer = purple_conversation_get_name(conversation);
    } else if (purple_conversation_get_type(conversation) ==
               PURPLE_CONV_TYPE_CHAT) {
        group_key = g_hash_table_lookup(
            connection->group_keys_by_id,
            GINT_TO_POINTER(purple_conv_chat_get_id(
                PURPLE_CONV_CHAT(conversation))));
    }

    for (guint index = connection->pending_reads->len; index > 0; index--) {
        SignalPendingRead *read = g_ptr_array_index(
            connection->pending_reads, index - 1);
        gboolean matches = read->chat_id != NULL
                               ? g_strcmp0(read->chat_id, group_key) == 0
                               : g_strcmp0(read->peer_id, direct_peer) == 0;
        if (matches && signal_send_read(connection, read))
            g_ptr_array_remove_index(connection->pending_reads, index - 1);
    }
}

static void
signal_identity_changed(SignalConnection *connection, const SignalEvent *event)
{
    SignalStatus status;

    if (event->peer_id == NULL || event->peer_id[0] == '\0' ||
        g_hash_table_contains(connection->identity_changes_seen,
                              event->peer_id))
        return;

    g_hash_table_add(connection->identity_changes_seen,
                     g_strdup(event->peer_id));
    if (event->value != 0) {
        g_hash_table_add(connection->pending_identity_changes,
                         g_strdup(event->peer_id));
        purple_notify_warning(
            connection, "Signal safety number changed",
            "Messages to a verified Signal contact are blocked",
            "Verify the contact through another channel, then right-click the buddy and choose “Accept changed Signal identity”.");
        return;
    }

    purple_notify_info(
        connection, "Signal safety number changed",
        "Communication continued for an unverified contact",
        "Verify the safety number in an official Signal client before sharing sensitive information.");
    status = signal_core_dismiss_identity(
        connection->core, connection->next_request_id++, event->peer_id);
    if (status != SIGNAL_STATUS_OK)
        purple_debug_warning(
            "signal-purple", "Could not dismiss identity notice: %d\n", status);
}

static void
signal_identity_accepted(SignalConnection *connection,
                         const SignalEvent *event)
{
    if (event->peer_id == NULL)
        return;
    g_hash_table_remove(connection->pending_identity_changes, event->peer_id);
    g_hash_table_remove(connection->identity_changes_seen, event->peer_id);
    purple_notify_info(connection, "Signal identity accepted",
                       "Messaging can continue",
                       "The contact is now unverified until its safety number is verified again.");
}

static gboolean
signal_group_leave_failed(SignalConnection *connection,
                          const SignalEvent *event)
{
    if (event->chat_id == NULL ||
        !g_hash_table_contains(connection->pending_group_leaves,
                               event->chat_id))
        return FALSE;

    g_hash_table_remove(connection->pending_group_leaves, event->chat_id);
    return TRUE;
}

static void
signal_group_left(SignalConnection *connection, const SignalEvent *event)
{
    g_autofree char *title = NULL;

    if (event->chat_id == NULL || event->chat_id[0] == '\0')
        return;

    title = g_strdup(signal_group_display_title(connection, event->chat_id));
    signal_deactivate_group(connection, event->chat_id, TRUE);
    purple_debug_info("signal-purple", "Completed remote Signal group leave\n");
    purple_notify_info(connection, "Signal group left",
                       title != NULL ? title : "Signal group",
                       "The group was left on Signal and removed from the chat list.");
}

static gboolean
signal_handle_event(SignalConnection *connection, const SignalEvent *event)
{
    switch ((SignalEventKind)event->kind) {
    case SIGNAL_EVENT_LINK_QR:
        signal_show_link_qr(connection, event);
        purple_connection_update_progress(connection->gc,
                                          "Waiting for your phone", 1, 3);
        break;
    case SIGNAL_EVENT_READY:
        purple_request_close_with_handle(&connection->link_qr);
        g_clear_pointer(&connection->link_qr, g_bytes_unref);
        purple_connection_update_progress(connection->gc,
                                          "Signal messages synchronized", 2, 3);
        purple_connection_set_state(connection->gc, PURPLE_CONNECTED);
        break;
    case SIGNAL_EVENT_CONTACT_SYNC_BEGIN:
        signal_begin_contact_sync(connection);
        break;
    case SIGNAL_EVENT_CONTACT:
        signal_add_contact(connection, event);
        break;
    case SIGNAL_EVENT_CONTACT_SYNC_END:
        signal_end_contact_sync(connection);
        break;
    case SIGNAL_EVENT_GROUP_SYNC_BEGIN:
        signal_begin_group_sync(connection);
        break;
    case SIGNAL_EVENT_GROUP:
        signal_add_group(connection, event);
        break;
    case SIGNAL_EVENT_GROUP_MEMBER:
        signal_add_group_member(connection, event);
        break;
    case SIGNAL_EVENT_GROUP_SYNC_END:
        signal_end_group_sync(connection);
        break;
    case SIGNAL_EVENT_MESSAGE:
        signal_deliver_direct(connection, event);
        break;
    case SIGNAL_EVENT_GROUP_MESSAGE:
        signal_deliver_group(connection, event);
        break;
    case SIGNAL_EVENT_GROUP_LEFT:
        signal_group_left(connection, event);
        break;
    case SIGNAL_EVENT_ATTACHMENT:
        signal_deliver_attachment(connection, event);
        break;
    case SIGNAL_EVENT_ATTACHMENT_SENT:
        signal_outgoing_attachment_complete(connection, event);
        break;
    case SIGNAL_EVENT_TYPING:
        if (event->peer_id != NULL) {
            if (event->value != 0)
                serv_got_typing(connection->gc, event->peer_id, 15,
                                PURPLE_TYPING);
            else
                serv_got_typing_stopped(connection->gc, event->peer_id);
        }
        break;
    case SIGNAL_EVENT_RECEIPT:
        /* Purple 2 has no safe per-message receipt update API. */
        break;
    case SIGNAL_EVENT_IDENTITY_CHANGE:
        signal_identity_changed(connection, event);
        break;
    case SIGNAL_EVENT_IDENTITY_ACCEPTED:
        signal_identity_accepted(connection, event);
        break;
    case SIGNAL_EVENT_NOTICE:
        purple_notify_info(connection, "signal-purple", event->title,
                           event->text);
        break;
    case SIGNAL_EVENT_ERROR:
        if ((event->flags & SIGNAL_EVENT_FLAG_FATAL) != 0) {
            purple_connection_error_reason(
                connection->gc, PURPLE_CONNECTION_ERROR_NETWORK_ERROR,
                event->text != NULL ? event->text : "Signal backend failed");
            return FALSE;
        }
        if (signal_group_leave_failed(connection, event)) {
            purple_debug_warning("signal-purple",
                                 "Remote Signal group leave failed\n");
            purple_notify_error(connection, "Could not leave Signal group",
                                "The group was not removed", event->text);
        } else if (!signal_outgoing_attachment_failed(connection, event))
            purple_notify_error(connection, "signal-purple", event->title,
                                event->text);
        break;
    case SIGNAL_EVENT_DISCONNECTED:
        purple_connection_error_reason(
            connection->gc, PURPLE_CONNECTION_ERROR_NETWORK_ERROR,
            event->text != NULL ? event->text
                                : "Signal connection ended unexpectedly");
        return FALSE;
    default:
        purple_debug_warning("signal-purple", "Unknown backend event %u\n",
                             event->kind);
        break;
    }

    return TRUE;
}

static gboolean
signal_poll_backend(gint fd, GIOCondition condition, gpointer data)
{
    SignalConnection *connection = data;
    gboolean notifier_failed =
        (condition & (G_IO_ERR | G_IO_HUP | G_IO_NVAL)) != 0;

    (void)fd;

    if (connection->closing || connection->core == NULL)
        return G_SOURCE_REMOVE;

    for (guint index = 0; index < 64; index++) {
        SignalEvent *event = NULL;
        int result = signal_core_poll_event(connection->core, &event);
        gboolean keep;

        if (result < 0) {
            purple_connection_error_reason(
                connection->gc, PURPLE_CONNECTION_ERROR_NETWORK_ERROR,
                "Signal backend event channel disconnected unexpectedly");
            return G_SOURCE_REMOVE;
        }
        if (result == 0) {
            if (notifier_failed) {
                purple_connection_error_reason(
                    connection->gc, PURPLE_CONNECTION_ERROR_NETWORK_ERROR,
                    "Signal backend event notifier disconnected unexpectedly");
                return G_SOURCE_REMOVE;
            }
            return G_SOURCE_CONTINUE;
        }

        if (event == NULL || event->abi_version != SIGNAL_CORE_ABI_VERSION ||
            event->struct_size < sizeof(SignalEvent)) {
            if (event != NULL)
                signal_event_free(event);
            purple_connection_error_reason(
                connection->gc, PURPLE_CONNECTION_ERROR_OTHER_ERROR,
                "Signal backend returned an incompatible event");
            return G_SOURCE_REMOVE;
        }

        keep = signal_handle_event(connection, event);
        if (keep && event->request_id != 0 &&
            (event->kind == SIGNAL_EVENT_MESSAGE ||
             event->kind == SIGNAL_EVENT_GROUP_MESSAGE ||
             event->kind == SIGNAL_EVENT_ATTACHMENT)) {
            SignalStatus status = signal_core_ack_message(
                connection->core, event->request_id);
            if (status != SIGNAL_STATUS_OK)
                purple_debug_warning(
                    "signal-purple",
                    "Could not queue message acknowledgment %" G_GUINT64_FORMAT
                    ": %d\n",
                    event->request_id, status);
        }
        signal_event_free(event);
        if (!keep)
            return G_SOURCE_REMOVE;
    }

    return G_SOURCE_CONTINUE;
}

static SignalConnection *
signal_connection_data(PurpleConnection *gc)
{
    return gc != NULL ? purple_connection_get_protocol_data(gc) : NULL;
}

void
signal_login(PurpleAccount *account)
{
    PurpleConnection *gc = purple_account_get_connection(account);
    SignalConnection *connection;
    SignalCoreConfig config = {
        .abi_version = SIGNAL_CORE_ABI_VERSION,
        .struct_size = sizeof(SignalCoreConfig),
    };
    SignalStatus status;
    g_autoptr(GError) error = NULL;
    g_autofree char *store_path = NULL;
    char *passphrase = NULL;
    GSource *source;
    int event_fd = -1;

    if (signal_core_abi_version() != SIGNAL_CORE_ABI_VERSION) {
        purple_connection_error_reason(gc, PURPLE_CONNECTION_ERROR_OTHER_ERROR,
                                       "signal-purple backend ABI mismatch");
        return;
    }

    store_path = signal_store_path(account, &error);
    if (store_path == NULL) {
        purple_connection_error_reason(gc, PURPLE_CONNECTION_ERROR_OTHER_ERROR,
                                       error->message);
        return;
    }

    passphrase = signal_secret_get_or_create(account, store_path, &error);
    if (passphrase == NULL) {
        purple_connection_error_reason(
            gc, PURPLE_CONNECTION_ERROR_AUTHENTICATION_FAILED,
            error != NULL ? error->message
                          : "Could not load the Signal database key from libsecret");
        return;
    }

    connection = g_new0(SignalConnection, 1);
    connection->gc = gc;
    connection->store_path = g_strdup(store_path);
    connection->group_ids_by_key = g_hash_table_new_full(
        g_str_hash, g_str_equal, g_free, NULL);
    connection->group_keys_by_id = g_hash_table_new_full(
        g_direct_hash, g_direct_equal, NULL, g_free);
    connection->group_titles_by_key = g_hash_table_new_full(
        g_str_hash, g_str_equal, g_free, g_free);
    connection->group_members_by_key = g_hash_table_new_full(
        g_str_hash, g_str_equal, g_free, (GDestroyNotify)g_ptr_array_unref);
    connection->active_group_keys = g_hash_table_new_full(
        g_str_hash, g_str_equal, g_free, NULL);
    connection->pending_group_leaves = g_hash_table_new_full(
        g_str_hash, g_str_equal, g_free, NULL);
    connection->identity_changes_seen = g_hash_table_new_full(
        g_str_hash, g_str_equal, g_free, NULL);
    connection->pending_identity_changes = g_hash_table_new_full(
        g_str_hash, g_str_equal, g_free, NULL);
    connection->outgoing_attachments = g_hash_table_new_full(
        g_int64_hash, g_int64_equal, g_free, (GDestroyNotify)purple_xfer_unref);
    connection->pending_reads = g_ptr_array_new_with_free_func(
        signal_pending_read_free);
    connection->group_leave_requests = g_ptr_array_new_with_free_func(
        signal_group_leave_request_free);
    signal_contact_sync_init(&connection->contact_sync);
    signal_contact_sync_init(&connection->group_sync);
    connection->next_group_id = 1;
    connection->next_request_id = 1;

    config.store_path = store_path;
    config.device_name = purple_account_get_string(account, "device-name",
                                                    "signal-purple");
    config.passphrase = passphrase;
    status = signal_core_new(&config, &connection->core);
    secret_password_free(passphrase);

    if (status == SIGNAL_STATUS_OK) {
        event_fd = signal_core_event_fd(connection->core);
        if (event_fd < 0) {
            signal_core_free(connection->core);
            connection->core = NULL;
            status = SIGNAL_STATUS_INTERNAL_ERROR;
        }
    }

    if (status != SIGNAL_STATUS_OK) {
        g_hash_table_unref(connection->group_ids_by_key);
        g_hash_table_unref(connection->group_keys_by_id);
        g_hash_table_unref(connection->group_titles_by_key);
        g_hash_table_unref(connection->group_members_by_key);
        g_hash_table_unref(connection->active_group_keys);
        g_hash_table_unref(connection->pending_group_leaves);
        g_hash_table_unref(connection->identity_changes_seen);
        g_hash_table_unref(connection->pending_identity_changes);
        g_hash_table_unref(connection->outgoing_attachments);
        g_ptr_array_unref(connection->pending_reads);
        g_ptr_array_unref(connection->group_leave_requests);
        signal_contact_sync_clear(&connection->contact_sync);
        signal_contact_sync_clear(&connection->group_sync);
        g_free(connection->store_path);
        g_free(connection);
        purple_connection_error_reason(gc, PURPLE_CONNECTION_ERROR_OTHER_ERROR,
                                       "Could not start the Signal backend");
        return;
    }

    purple_connection_set_protocol_data(gc, connection);
    purple_signal_connect(
        purple_conversations_get_handle(), "conversation-updated", connection,
        PURPLE_CALLBACK(signal_conversation_updated), connection);
    purple_connection_set_state(gc, PURPLE_CONNECTING);
    purple_connection_update_progress(gc, "Opening encrypted Signal store", 0, 3);

    source = g_unix_fd_source_new(event_fd,
                                  G_IO_IN | G_IO_ERR | G_IO_HUP | G_IO_NVAL);
    g_source_set_callback(source, G_SOURCE_FUNC(signal_poll_backend), connection,
                          NULL);
    connection->poll_source = source;
    if (g_source_attach(source, g_main_context_default()) == 0) {
        connection->poll_source = NULL;
        g_source_unref(source);
        purple_connection_error_reason(
            gc, PURPLE_CONNECTION_ERROR_OTHER_ERROR,
            "Could not attach the Signal backend event notifier");
    }
}

void
signal_close(PurpleConnection *gc)
{
    SignalConnection *connection = signal_connection_data(gc);

    if (connection == NULL)
        return;

    connection->closing = TRUE;
    purple_connection_set_protocol_data(gc, NULL);
    purple_signals_disconnect_by_handle(connection);
    purple_request_close_with_handle(&connection->link_qr);
    purple_request_close_with_handle(connection);

    for (guint index = 0; index < connection->group_leave_requests->len;
         index++) {
        SignalGroupLeaveRequest *request = g_ptr_array_index(
            connection->group_leave_requests, index);

        request->connection = NULL;
    }

    GHashTableIter attachment_iter;
    gpointer attachment_value;
    g_hash_table_iter_init(&attachment_iter,
                           connection->outgoing_attachments);
    while (g_hash_table_iter_next(&attachment_iter, NULL, &attachment_value)) {
        PurpleXfer *xfer = attachment_value;
        SignalOutgoingAttachment *attachment = xfer->data;
        if (attachment != NULL)
            attachment->connection = NULL;
    }
    g_hash_table_remove_all(connection->outgoing_attachments);

    if (connection->poll_source != NULL) {
        g_source_destroy(connection->poll_source);
        g_source_unref(connection->poll_source);
    }
    if (connection->core != NULL)
        signal_core_free(connection->core);

    g_clear_pointer(&connection->link_qr, g_bytes_unref);
    g_hash_table_unref(connection->group_ids_by_key);
    g_hash_table_unref(connection->group_keys_by_id);
    g_hash_table_unref(connection->group_titles_by_key);
    g_hash_table_unref(connection->group_members_by_key);
    g_hash_table_unref(connection->active_group_keys);
    g_hash_table_unref(connection->pending_group_leaves);
    g_hash_table_unref(connection->identity_changes_seen);
    g_hash_table_unref(connection->pending_identity_changes);
    g_hash_table_unref(connection->outgoing_attachments);
    g_ptr_array_unref(connection->pending_reads);
    g_ptr_array_unref(connection->group_leave_requests);
    signal_contact_sync_clear(&connection->contact_sync);
    signal_contact_sync_clear(&connection->group_sync);
    g_free(connection->store_path);
    g_free(connection);
}

static void
signal_accept_changed_identity(PurpleBlistNode *node, gpointer user_data)
{
    PurpleBuddy *buddy;
    PurpleAccount *account;
    PurpleConnection *gc;
    SignalConnection *connection;
    const char *peer;
    SignalStatus status;

    (void)user_data;
    if (!PURPLE_BLIST_NODE_IS_BUDDY(node))
        return;
    buddy = PURPLE_BUDDY(node);
    account = purple_buddy_get_account(buddy);
    gc = purple_account_get_connection(account);
    connection = signal_connection_data(gc);
    peer = purple_buddy_get_name(buddy);
    if (connection == NULL || connection->closing ||
        !g_hash_table_contains(connection->pending_identity_changes, peer))
        return;

    status = signal_core_accept_identity(
        connection->core, connection->next_request_id++, peer);
    if (status != SIGNAL_STATUS_OK)
        purple_notify_error(connection, "Could not accept Signal identity",
                            "The request could not be queued",
                            "Reconnect the account and try again.");
}

static void
signal_cancel_group_leave(void *user_data, int action)
{
    SignalGroupLeaveRequest *request = user_data;

    (void)action;
    signal_finish_group_leave_request(request);
}

static void
signal_confirm_group_leave(void *user_data, int action)
{
    SignalGroupLeaveRequest *request = user_data;
    SignalConnection *connection =
        request != NULL ? request->connection : NULL;
    SignalStatus status;

    (void)action;
    if (connection == NULL || connection->closing ||
        !signal_group_is_active(connection, request->group_key)) {
        if (connection != NULL && !connection->closing)
            purple_notify_error(connection, "Signal group unavailable",
                                "The group is no longer active",
                                "Reconnect and check the Signal group list.");
        signal_finish_group_leave_request(request);
        return;
    }
    if (g_hash_table_contains(connection->pending_group_leaves,
                              request->group_key)) {
        signal_finish_group_leave_request(request);
        return;
    }

    status = signal_core_leave_group(connection->core,
                                     connection->next_request_id++,
                                     request->group_key);
    if (status == SIGNAL_STATUS_OK) {
        g_hash_table_add(connection->pending_group_leaves,
                         g_strdup(request->group_key));
        purple_debug_info("signal-purple", "Queued remote Signal group leave\n");
        purple_notify_info(connection, "Leaving Signal group",
                           request->title,
                           "The chat will be removed after Signal confirms the leave.");
    } else {
        purple_notify_error(connection, "Could not leave Signal group",
                            "The request could not be queued",
                            "Reconnect the account and try again.");
    }
    signal_finish_group_leave_request(request);
}

static void
signal_request_group_leave(PurpleBlistNode *node, gpointer user_data)
{
    PurpleChat *chat;
    PurpleAccount *account;
    PurpleConnection *gc;
    SignalConnection *connection;
    SignalGroupLeaveRequest *request;
    const char *group_key;
    const char *title;
    g_autofree char *primary = NULL;

    (void)user_data;
    if (!PURPLE_BLIST_NODE_IS_CHAT(node) ||
        !purple_blist_node_get_bool(node, SIGNAL_SYNCED_GROUP_KEY))
        return;

    chat = PURPLE_CHAT(node);
    account = purple_chat_get_account(chat);
    gc = purple_account_get_connection(account);
    connection = signal_connection_data(gc);
    group_key = signal_group_sync_chat_id(chat);
    if (connection == NULL || connection->closing ||
        !signal_group_is_active(connection, group_key) ||
        g_hash_table_contains(connection->pending_group_leaves, group_key) ||
        signal_group_leave_prompted(connection, group_key))
        return;

    title = purple_chat_get_name(chat);
    request = g_new0(SignalGroupLeaveRequest, 1);
    request->connection = connection;
    request->group_key = g_strdup(group_key);
    request->title = g_strdup(title != NULL && title[0] != '\0'
                                  ? title
                                  : signal_group_title(connection, group_key));
    g_ptr_array_add(connection->group_leave_requests, request);
    primary = g_strdup_printf("Leave “%s” on Signal?", request->title);

    if (purple_request_action(
            connection, "Leave Signal group", primary,
            "You will stop receiving messages from this group. This does not delete the group for its other members.",
            0, account, NULL, NULL, request, 2,
            "_Cancel", G_CALLBACK(signal_cancel_group_leave),
            "_Leave group", G_CALLBACK(signal_confirm_group_leave)) == NULL)
        signal_finish_group_leave_request(request);
}

GList *
signal_blist_node_menu(PurpleBlistNode *node)
{
    PurpleBuddy *buddy;
    PurpleAccount *account;
    PurpleConnection *gc;
    SignalConnection *connection;

    if (PURPLE_BLIST_NODE_IS_CHAT(node)) {
        PurpleChat *chat = PURPLE_CHAT(node);
        const char *group_key;

        account = purple_chat_get_account(chat);
        if (g_strcmp0(purple_account_get_protocol_id(account),
                      SIGNAL_PLUGIN_ID) != 0 ||
            !purple_blist_node_get_bool(node, SIGNAL_SYNCED_GROUP_KEY))
            return NULL;
        gc = purple_account_get_connection(account);
        connection = signal_connection_data(gc);
        group_key = signal_group_sync_chat_id(chat);
        if (connection == NULL || connection->closing ||
            !signal_group_is_active(connection, group_key) ||
            g_hash_table_contains(connection->pending_group_leaves,
                                  group_key) ||
            signal_group_leave_prompted(connection, group_key))
            return NULL;

        return g_list_append(
            NULL,
            purple_menu_action_new(
                "Leave Signal group…",
                PURPLE_CALLBACK(signal_request_group_leave), NULL, NULL));
    }

    if (!PURPLE_BLIST_NODE_IS_BUDDY(node))
        return NULL;
    buddy = PURPLE_BUDDY(node);
    account = purple_buddy_get_account(buddy);
    if (g_strcmp0(purple_account_get_protocol_id(account), SIGNAL_PLUGIN_ID) != 0)
        return NULL;
    gc = purple_account_get_connection(account);
    connection = signal_connection_data(gc);
    if (connection == NULL ||
        !g_hash_table_contains(connection->pending_identity_changes,
                               purple_buddy_get_name(buddy)))
        return NULL;

    return g_list_append(
        NULL,
        purple_menu_action_new(
            "Accept changed Signal identity",
            PURPLE_CALLBACK(signal_accept_changed_identity), NULL, NULL));
}

static int
signal_status_to_errno(SignalStatus status)
{
    switch (status) {
    case SIGNAL_STATUS_OK:
        return 1;
    case SIGNAL_STATUS_NOT_READY:
        return -ENOTCONN;
    case SIGNAL_STATUS_QUEUE_FULL:
        return -EAGAIN;
    case SIGNAL_STATUS_INVALID_ARGUMENT:
        return -EINVAL;
    default:
        return -EIO;
    }
}

int
signal_send_im(PurpleConnection *gc, const char *who, const char *message,
               PurpleMessageFlags flags)
{
    SignalConnection *connection = signal_connection_data(gc);
    PurpleAccount *account;
    PurpleConversation *conversation;
    g_autofree char *plain = NULL;
    SignalStatus status;

    (void)flags;
    if (connection == NULL || connection->closing)
        return -ENOTCONN;

    plain = signal_plaintext_from_markup(message);
    if (plain == NULL)
        return -EINVAL;
    if (strlen(plain) > SIGNAL_MAX_MESSAGE_BYTES)
        return -E2BIG;

    account = purple_connection_get_account(gc);
    conversation = purple_find_conversation_with_account(
        PURPLE_CONV_TYPE_IM, who, account);
    if (conversation == NULL)
        conversation = purple_conversation_new(PURPLE_CONV_TYPE_IM, account,
                                               who);
    purple_conversation_set_logging(conversation, FALSE);

    status = signal_core_send_message(connection->core,
                                      connection->next_request_id++, who, plain);
    return signal_status_to_errno(status);
}

unsigned int
signal_send_typing(PurpleConnection *gc, const char *who,
                   PurpleTypingState state)
{
    SignalConnection *connection = signal_connection_data(gc);

    if (connection == NULL || connection->closing)
        return 0;

    signal_core_set_typing(connection->core, connection->next_request_id++, who,
                           state == PURPLE_TYPING);
    return 0;
}

static PurpleXfer *
signal_new_attachment_xfer(SignalConnection *connection,
                           const char *display_peer, const char *recipient,
                           gboolean group)
{
    PurpleXfer *xfer;
    SignalOutgoingAttachment *attachment;

    if (connection == NULL || connection->closing || display_peer == NULL ||
        recipient == NULL)
        return NULL;
    xfer = purple_xfer_new(purple_connection_get_account(connection->gc),
                           PURPLE_XFER_SEND, display_peer);
    if (xfer == NULL)
        return NULL;
    attachment = g_new0(SignalOutgoingAttachment, 1);
    attachment->connection = connection;
    attachment->recipient = g_strdup(recipient);
    attachment->group = group;
    xfer->data = attachment;
    purple_xfer_set_init_fnc(xfer, signal_outgoing_attachment_init);
    purple_xfer_set_end_fnc(xfer, signal_outgoing_attachment_free);
    purple_xfer_set_request_denied_fnc(xfer,
                                       signal_outgoing_attachment_free);
    purple_xfer_set_cancel_send_fnc(xfer,
                                    signal_outgoing_attachment_cancel);
    return xfer;
}

gboolean
signal_can_receive_file(PurpleConnection *gc, const char *who)
{
    SignalConnection *connection = signal_connection_data(gc);

    return connection != NULL && !connection->closing && who != NULL &&
           who[0] != '\0';
}

PurpleXfer *
signal_new_xfer(PurpleConnection *gc, const char *who)
{
    SignalConnection *connection = signal_connection_data(gc);

    return signal_new_attachment_xfer(connection, who, who, FALSE);
}

void
signal_send_file(PurpleConnection *gc, const char *who, const char *filename)
{
    PurpleXfer *xfer = signal_new_xfer(gc, who);

    if (xfer == NULL)
        return;
    if (filename != NULL)
        purple_xfer_request_accepted(xfer, filename);
    else
        purple_xfer_request(xfer);
}

GList *
signal_chat_info(PurpleConnection *gc)
{
    struct proto_chat_entry *group_key = g_new0(struct proto_chat_entry, 1);

    (void)gc;
    group_key->label = "Signal group identifier";
    group_key->identifier = SIGNAL_GROUP_COMPONENT_KEY;
    group_key->required = TRUE;
    return g_list_append(NULL, group_key);
}

GHashTable *
signal_chat_info_defaults(PurpleConnection *gc, const char *chat_name)
{
    GHashTable *components = g_hash_table_new_full(
        g_str_hash, g_str_equal, NULL, g_free);

    (void)gc;
    if (chat_name != NULL && chat_name[0] != '\0')
        g_hash_table_insert(components, SIGNAL_GROUP_COMPONENT_KEY,
                            g_strdup(chat_name));
    return components;
}

char *
signal_get_chat_name(GHashTable *components)
{
    const char *group_key = components != NULL
                                ? g_hash_table_lookup(
                                      components, SIGNAL_GROUP_COMPONENT_KEY)
                                : NULL;
    if (group_key == NULL && components != NULL)
        group_key = g_hash_table_lookup(components,
                                        SIGNAL_LEGACY_GROUP_COMPONENT_KEY);
    return g_strdup(group_key != NULL ? group_key : "");
}

void
signal_join_chat(PurpleConnection *gc, GHashTable *components)
{
    SignalConnection *connection = signal_connection_data(gc);
    const char *group_key;

    if (connection == NULL || connection->closing || components == NULL)
        return;
    group_key = g_hash_table_lookup(components, SIGNAL_GROUP_COMPONENT_KEY);
    if (group_key == NULL)
        group_key = g_hash_table_lookup(components,
                                        SIGNAL_LEGACY_GROUP_COMPONENT_KEY);
    if (!signal_group_can_send(connection, group_key)) {
        purple_notify_error(connection, "Signal group unavailable",
                            "This Signal group is inactive or being left",
                            "Wait for a pending leave or reconnect after confirming membership on another Signal device.");
        return;
    }
    signal_open_group(connection, group_key, NULL);
}

void
signal_chat_leave(PurpleConnection *gc, int id)
{
    serv_got_chat_left(gc, id);
}

int
signal_chat_send(PurpleConnection *gc, int id, const char *message,
                 PurpleMessageFlags flags)
{
    SignalConnection *connection = signal_connection_data(gc);
    PurpleConversation *conversation;
    const char *group_key;
    g_autofree char *plain = NULL;
    SignalStatus status;

    (void)flags;
    if (connection == NULL || connection->closing)
        return -ENOTCONN;

    group_key = g_hash_table_lookup(connection->group_keys_by_id,
                                    GINT_TO_POINTER(id));
    if (!signal_group_can_send(connection, group_key))
        return -ENOENT;

    plain = signal_plaintext_from_markup(message);
    if (plain == NULL)
        return -EINVAL;
    if (strlen(plain) > SIGNAL_MAX_MESSAGE_BYTES)
        return -E2BIG;

    conversation = purple_find_chat(gc, id);
    if (conversation != NULL)
        purple_conversation_set_logging(conversation, FALSE);

    status = signal_core_send_group_message(
        connection->core, connection->next_request_id++, group_key, plain);
    return signal_status_to_errno(status);
}

gboolean
signal_chat_can_receive_file(PurpleConnection *gc, int id)
{
    SignalConnection *connection = signal_connection_data(gc);
    const char *group_key;

    if (connection == NULL || connection->closing)
        return FALSE;
    group_key = g_hash_table_lookup(connection->group_keys_by_id,
                                    GINT_TO_POINTER(id));
    return signal_group_can_send(connection, group_key);
}

void
signal_chat_send_file(PurpleConnection *gc, int id, const char *filename)
{
    SignalConnection *connection = signal_connection_data(gc);
    const char *group_key;
    PurpleXfer *xfer;

    if (!signal_chat_can_receive_file(gc, id))
        return;
    group_key = g_hash_table_lookup(connection->group_keys_by_id,
                                    GINT_TO_POINTER(id));
    xfer = signal_new_attachment_xfer(
        connection, signal_group_title(connection, group_key), group_key, TRUE);
    if (xfer == NULL)
        return;
    if (filename != NULL)
        purple_xfer_request_accepted(xfer, filename);
    else
        purple_xfer_request(xfer);
}
