/* SPDX-License-Identifier: GPL-3.0-or-later */
#include "signal_purple.h"

#include <errno.h>
#include <libsecret/secret.h>

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

    purple_request_close_with_handle(connection);
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
        connection, "Link signal-purple", "Scan this QR code with Signal",
        "On your phone, open Signal Settings > Linked devices > Link new device.",
        fields, "I've scanned it", G_CALLBACK(signal_link_keep_waiting),
        "Cancel linking", G_CALLBACK(signal_link_cancel),
        purple_connection_get_account(connection->gc), NULL, NULL,
        connection);
}

static void
signal_add_contact(SignalConnection *connection, const SignalEvent *event)
{
    PurpleAccount *account;
    PurpleBuddy *buddy;
    PurpleGroup *group;

    if (event->peer_id == NULL || event->peer_id[0] == '\0')
        return;

    account = purple_connection_get_account(connection->gc);
    buddy = purple_find_buddy(account, event->peer_id);
    if (buddy == NULL) {
        group = purple_find_group("Signal");
        if (group == NULL) {
            group = purple_group_new("Signal");
            purple_blist_add_group(group, NULL);
        }
        buddy = purple_buddy_new(account, event->peer_id, NULL);
        purple_blist_add_buddy(buddy, NULL, group, NULL);
    }

    if (event->title != NULL && event->title[0] != '\0')
        purple_blist_server_alias_buddy(buddy, event->title);
}

static PurpleConversation *
signal_open_group(SignalConnection *connection, const char *group_key,
                  const char *title)
{
    PurpleConversation *conversation;
    guint id = signal_group_id(connection, group_key, title);

    conversation = purple_find_chat(connection->gc, (int)id);
    if (conversation == NULL)
        conversation = serv_got_joined_chat(connection->gc, (int)id,
                                             signal_group_title(connection,
                                                                group_key));
    purple_conversation_set_logging(conversation, FALSE);
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
                    PURPLE_MESSAGE_RECV | PURPLE_MESSAGE_NO_LOG, timestamp);
        return;
    }

    purple_conv_im_write(PURPLE_CONV_IM(conversation),
                         purple_account_get_username(account), escaped,
                         PURPLE_MESSAGE_SEND | PURPLE_MESSAGE_REMOTE_SEND |
                             PURPLE_MESSAGE_NO_LOG,
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

    signal_open_group(connection, event->chat_id, event->title);
    id = signal_group_id(connection, event->chat_id, event->title);
    escaped = g_markup_escape_text(event->text, -1);
    timestamp = event->timestamp_ms > 0
                    ? (time_t)(event->timestamp_ms / 1000)
                    : time(NULL);
    serv_got_chat_in(connection->gc, (int)id, event->peer_id,
                     (event->flags & SIGNAL_EVENT_FLAG_OUTGOING)
                         ? PURPLE_MESSAGE_SEND | PURPLE_MESSAGE_REMOTE_SEND |
                               PURPLE_MESSAGE_NO_LOG
                         : PURPLE_MESSAGE_RECV | PURPLE_MESSAGE_NO_LOG,
                     escaped, timestamp);
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
        purple_request_close_with_handle(connection);
        g_clear_pointer(&connection->link_qr, g_bytes_unref);
        purple_connection_update_progress(connection->gc,
                                          "Signal messages synchronized", 3, 3);
        purple_connection_set_state(connection->gc, PURPLE_CONNECTED);
        break;
    case SIGNAL_EVENT_CONTACT:
        signal_add_contact(connection, event);
        break;
    case SIGNAL_EVENT_GROUP:
        if (event->chat_id != NULL)
            signal_group_id(connection, event->chat_id, event->title);
        break;
    case SIGNAL_EVENT_MESSAGE:
        signal_deliver_direct(connection, event);
        break;
    case SIGNAL_EVENT_GROUP_MESSAGE:
        signal_deliver_group(connection, event);
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
signal_poll_backend(gpointer data)
{
    SignalConnection *connection = data;

    if (connection->closing || connection->core == NULL)
        return G_SOURCE_REMOVE;

    for (guint index = 0; index < 64; index++) {
        SignalEvent *event = NULL;
        int result = signal_core_poll_event(connection->core, &event);
        gboolean keep;

        if (result <= 0)
            return result == 0 ? G_SOURCE_CONTINUE : G_SOURCE_REMOVE;

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
    connection->next_group_id = 1;
    connection->next_request_id = 1;

    config.store_path = store_path;
    config.device_name = purple_account_get_string(account, "device-name",
                                                    "signal-purple");
    config.passphrase = passphrase;
    status = signal_core_new(&config, &connection->core);
    secret_password_free(passphrase);

    if (status != SIGNAL_STATUS_OK) {
        g_hash_table_unref(connection->group_ids_by_key);
        g_hash_table_unref(connection->group_keys_by_id);
        g_hash_table_unref(connection->group_titles_by_key);
        g_free(connection->store_path);
        g_free(connection);
        purple_connection_error_reason(gc, PURPLE_CONNECTION_ERROR_OTHER_ERROR,
                                       "Could not start the Signal backend");
        return;
    }

    purple_connection_set_protocol_data(gc, connection);
    purple_connection_set_state(gc, PURPLE_CONNECTING);
    purple_connection_update_progress(gc, "Opening encrypted Signal store", 0, 3);

    source = g_timeout_source_new(20);
    g_source_set_callback(source, signal_poll_backend, connection, NULL);
    g_source_attach(source, g_main_context_default());
    connection->poll_source = source;
}

void
signal_close(PurpleConnection *gc)
{
    SignalConnection *connection = signal_connection_data(gc);

    if (connection == NULL)
        return;

    connection->closing = TRUE;
    purple_connection_set_protocol_data(gc, NULL);
    purple_request_close_with_handle(connection);

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
    g_free(connection->store_path);
    g_free(connection);
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
    if (group_key == NULL)
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
