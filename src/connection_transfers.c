/* SPDX-License-Identifier: GPL-3.0-or-later */
#include "attachment_file.h"
#include "inline_image.h"
#include "signal_purple_internal.h"

#include <errno.h>

static void
signal_outgoing_attachment_destroy(SignalOutgoingAttachment *attachment)
{
    g_free(attachment->recipient);
    g_free(attachment);
}

void
signal_outgoing_attachment_detach(SignalOutgoingAttachment *attachment)
{
    for (GList *node = purple_xfers_get_all(); node != NULL;
         node = node->next) {
        PurpleXfer *xfer = node->data;

        if (xfer->data == attachment) {
            xfer->data = NULL;
            break;
        }
    }
    attachment->connection = NULL;
    signal_outgoing_attachment_destroy(attachment);
}

static void
signal_outgoing_attachment_free(PurpleXfer *xfer)
{
    SignalOutgoingAttachment *attachment;

    if (xfer == NULL || xfer->data == NULL)
        return;
    attachment = xfer->data;
    xfer->data = NULL;
    if (attachment->connection != NULL) {
        g_hash_table_remove(
            attachment->connection->outgoing_attachment_contexts, attachment);
        if (attachment->request_id != 0)
            g_hash_table_remove(attachment->connection->outgoing_attachments,
                                &attachment->request_id);
    }
    signal_outgoing_attachment_destroy(attachment);
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
    const char *local_filename;
    g_autofree char *filename = NULL;
    g_autofree char *mime_type = NULL;
    g_autoptr(GError) error = NULL;
    gsize size = 0;
    SignalStatus status;
    guint64 *key;

    if (attachment == NULL || attachment->connection == NULL ||
        attachment->connection->closing) {
        purple_xfer_cancel_local(xfer);
        return;
    }
    if (attachment->group &&
        !signal_group_can_send(attachment->connection,
                               attachment->recipient)) {
        purple_xfer_error(PURPLE_XFER_SEND, purple_xfer_get_account(xfer),
                          purple_xfer_get_remote_user(xfer),
                          "This Signal group is no longer active");
        purple_xfer_cancel_local(xfer);
        return;
    }
    local_filename = purple_xfer_get_local_filename(xfer);
    if (!signal_inspect_attachment_file(local_filename,
                                        SIGNAL_CORE_MAX_ATTACHMENT_BYTES,
                                        &size, &mime_type, &error)) {
        const char *message = error->message;

        if (g_error_matches(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA) ||
            g_error_matches(error, G_IO_ERROR,
                            G_IO_ERROR_MESSAGE_TOO_LARGE))
            message = "Signal attachments must be between 1 byte and 25 MiB";
        purple_xfer_error(PURPLE_XFER_SEND, purple_xfer_get_account(xfer),
                          purple_xfer_get_remote_user(xfer), message);
        purple_xfer_cancel_local(xfer);
        return;
    }
    purple_xfer_set_size(xfer, size);
    filename = g_path_get_basename(local_filename);

    attachment->request_id = attachment->connection->next_request_id++;
    purple_xfer_ref(xfer);
    attachment->connection->start_xfer(xfer, -1, NULL, 0);
    if (xfer->data == NULL) {
        purple_xfer_unref(xfer);
        return;
    }
    attachment = xfer->data;
    if (attachment->group) {
        status = signal_core_send_group_file_attachment(
            attachment->connection->core, attachment->request_id,
            attachment->recipient, filename, mime_type,
            local_filename);
    } else {
        status = signal_core_send_file_attachment(
            attachment->connection->core, attachment->request_id,
            attachment->recipient, filename, mime_type,
            local_filename);
    }
    if (status != SIGNAL_STATUS_OK) {
        attachment->request_id = 0;
        purple_xfer_error(PURPLE_XFER_SEND, purple_xfer_get_account(xfer),
                          purple_xfer_get_remote_user(xfer),
                          "The Signal attachment could not be queued");
        purple_xfer_cancel_local(xfer);
        purple_xfer_unref(xfer);
        return;
    }

    key = g_new(guint64, 1);
    *key = attachment->request_id;
    purple_xfer_ref(xfer);
    g_hash_table_insert(attachment->connection->outgoing_attachments, key,
                        xfer);
    purple_xfer_unref(xfer);
}

gboolean
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

gboolean
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

gboolean
signal_deliver_attachment(SignalConnection *connection,
                          const SignalEvent *event)
{
    PurpleAccount *account;
    PurpleConversation *conversation = NULL;
    PurpleXfer *xfer;
    SignalAttachment *attachment;
    g_autofree char *filename = NULL;
    const char *peer;
    time_t timestamp;

    if (event->peer_id == NULL || event->peer_id[0] == '\0' ||
        (event->chat_id != NULL && event->chat_id[0] == '\0') ||
        event->data == NULL || event->data_len == 0 ||
        event->data_len > 50 * 1024 * 1024) {
        purple_debug_warning(
            "signal-purple",
            "Rejected a malformed or oversized Signal attachment projection\n");
        return FALSE;
    }

    if (event->chat_id != NULL && event->chat_id[0] != '\0') {
        g_hash_table_add(connection->active_group_keys,
                         g_strdup(event->chat_id));
        conversation = signal_open_group(connection, event->chat_id, NULL);
    }

    account = purple_connection_get_account(connection->gc);
    if (!signal_event_can_present(connection, conversation, event)) {
        return TRUE;
    }

    peer = event->peer_id;
    filename = g_path_get_basename(
        event->title != NULL && event->title[0] != '\0'
            ? event->title
            : "signal-attachment");
    if (g_str_equal(filename, ".") || g_str_equal(filename, "..") ||
        g_str_equal(filename, G_DIR_SEPARATOR_S)) {
        g_free(g_steal_pointer(&filename));
        filename = g_strdup("signal-attachment");
    }

    timestamp = event->timestamp_ms > 0 ? (time_t)(event->timestamp_ms / 1000)
                                        : time(NULL);
    if (event->chat_id != NULL) {
        if (conversation != NULL &&
            signal_inline_image_deliver_group(
                connection->gc,
                purple_conv_chat_get_id(PURPLE_CONV_CHAT(conversation)), peer,
                filename, event->text, event->data, event->data_len,
                timestamp)) {
            signal_queue_final_read(connection, conversation, event);
            return TRUE;
        }
    } else {
        gboolean delivered = signal_inline_image_deliver_direct(
            connection->gc, peer, filename, event->text, event->data,
            event->data_len, timestamp);

        conversation = purple_find_conversation_with_account(
            PURPLE_CONV_TYPE_IM, peer, account);
        if (delivered) {
            signal_queue_final_read(connection, conversation, event);
            return TRUE;
        }
    }

    xfer = purple_xfer_new(account, PURPLE_XFER_RECEIVE, peer);
    if (xfer == NULL) {
        purple_notify_error(connection, "Signal attachment unavailable",
                            "Could not create a receive transfer",
                            "Restart Pidgin, then ask the sender to resend the attachment.");
        signal_queue_final_read(connection, conversation, event);
        return TRUE;
    }
    attachment = g_new0(SignalAttachment, 1);
    attachment->bytes = g_bytes_new(event->data, event->data_len);
    attachment->size = event->data_len;
    xfer->data = attachment;
    purple_xfer_set_filename(xfer, filename);
    purple_xfer_set_size(xfer, event->data_len);
    purple_xfer_set_init_fnc(xfer, signal_attachment_init);
    purple_xfer_set_start_fnc(xfer, signal_attachment_start);
    purple_xfer_set_end_fnc(xfer, signal_attachment_free);
    purple_xfer_set_request_denied_fnc(xfer, signal_attachment_free);
    purple_xfer_set_cancel_recv_fnc(xfer, signal_attachment_cancel);
    purple_xfer_request(xfer);
    signal_queue_final_read(connection, conversation, event);
    return TRUE;
}

PurpleXfer *
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
    g_hash_table_add(connection->outgoing_attachment_contexts, attachment);
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
