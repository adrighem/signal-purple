/* SPDX-License-Identifier: GPL-3.0-or-later */
#include <glib.h>
#include <glib/gstdio.h>
#include <gmodule.h>
#include <purple.h>
#include <errno.h>
#include <stdio.h>
#include <sys/stat.h>

#include "signal_purple.h"

G_STATIC_ASSERT(SIGNAL_CORE_ABI_VERSION == 7u);

typedef struct {
    PurpleInputFunction function;
    gpointer user_data;
} InputClosure;

typedef gboolean (*SignalHandleEventFunc)(SignalConnection *connection,
                                          const SignalEvent *event);
typedef gboolean (*SignalDispatchEventFunc)(SignalConnection *connection,
                                            const SignalEvent *event,
                                            gboolean *accepted);
typedef SignalConnection *(*SignalConnectionNewFunc)(PurpleConnection *gc,
                                                     const char *store_path);

static SignalConnectionNewFunc connection_new;

static gboolean
input_dispatch(GIOChannel *channel, GIOCondition condition, gpointer data)
{
    InputClosure *closure = data;
    PurpleInputCondition purple_condition = 0;

    if ((condition & (G_IO_IN | G_IO_HUP | G_IO_ERR)) != 0)
        purple_condition |= PURPLE_INPUT_READ;
    if ((condition & (G_IO_OUT | G_IO_HUP | G_IO_ERR)) != 0)
        purple_condition |= PURPLE_INPUT_WRITE;
    closure->function(closure->user_data, g_io_channel_unix_get_fd(channel),
                      purple_condition);
    return TRUE;
}

static guint
input_add(int fd, PurpleInputCondition condition, PurpleInputFunction function,
          gpointer user_data)
{
    GIOCondition gio_condition = 0;
    GIOChannel *channel;
    InputClosure *closure;
    guint source;

    if ((condition & PURPLE_INPUT_READ) != 0)
        gio_condition |= G_IO_IN | G_IO_HUP | G_IO_ERR;
    if ((condition & PURPLE_INPUT_WRITE) != 0)
        gio_condition |= G_IO_OUT | G_IO_HUP | G_IO_ERR;

    closure = g_new(InputClosure, 1);
    closure->function = function;
    closure->user_data = user_data;
    channel = g_io_channel_unix_new(fd);
    source = g_io_add_watch_full(channel, G_PRIORITY_DEFAULT, gio_condition,
                                 input_dispatch, closure, g_free);
    g_io_channel_unref(channel);
    return source;
}

static PurpleEventLoopUiOps event_loop_ops = {
    .timeout_add = g_timeout_add,
    .timeout_remove = g_source_remove,
    .input_add = input_add,
    .input_remove = g_source_remove,
    .timeout_add_seconds = g_timeout_add_seconds,
};

static GHashTable *
get_ui_info(void)
{
    static GHashTable *info;

    if (info == NULL) {
        info = g_hash_table_new(g_str_hash, g_str_equal);
        g_hash_table_insert(info, "name", "signal-purple tests");
        g_hash_table_insert(info, "version", "0");
    }
    return info;
}

static PurpleCoreUiOps core_ops = {
    .get_ui_info = get_ui_info,
};

static guint member_update_title_autosets;

static void
autoset_title_after_chat_remove(PurpleConversation *conversation,
                                GList *users)
{
    (void)users;
    member_update_title_autosets++;
    purple_conversation_autoset_title(conversation);
}

static PurpleConversationUiOps reconnect_conversation_ops = {
    .chat_remove_users = autoset_title_after_chat_remove,
};

static void
count_chat_joined(PurpleConversation *conversation, gpointer data)
{
    guint *count = data;

    (void)conversation;
    (*count)++;
}

static void
dispatch_group_snapshot(SignalHandleEventFunc handle_event,
                        SignalConnection *connection)
{
    SignalEvent events[] = {
        { .kind = SIGNAL_EVENT_GROUP_SYNC_BEGIN },
        {
            .kind = SIGNAL_EVENT_GROUP,
            .chat_id = "stable-conversation-one",
            .title = "Remote title one",
        },
        {
            .kind = SIGNAL_EVENT_GROUP_MEMBER,
            .chat_id = "stable-conversation-one",
            .peer_id = "aci:11111111-1111-4111-8111-111111111111",
        },
        {
            .kind = SIGNAL_EVENT_GROUP,
            .chat_id = "stable-conversation-two",
            .title = "Remote title two",
        },
        {
            .kind = SIGNAL_EVENT_GROUP,
            .chat_id = "stable-conversation-legacy",
            .title = "Remote legacy title",
        },
        { .kind = SIGNAL_EVENT_GROUP_SYNC_END },
    };

    for (guint index = 0; index < G_N_ELEMENTS(events); index++) {
        events[index].abi_version = SIGNAL_CORE_ABI_VERSION;
        events[index].struct_size = sizeof(SignalEvent);
        g_assert_true(handle_event(connection, &events[index]));
    }
}

typedef struct {
    SignalStatus status;
    guint calls;
    guint64 request_id;
    char *group_key;
    char *message;
} FakeGroupSender;

typedef struct {
    SignalStatus status;
    guint calls;
    guint64 request_id;
    guint64 timestamp;
    char *recipient;
} FakeReadMarker;

static gboolean read_conversation_focused;

static gboolean
read_has_focus(PurpleConversation *conversation)
{
    (void)conversation;
    return read_conversation_focused;
}

static PurpleConversationUiOps read_conversation_ops = {
    .has_focus = read_has_focus,
};

static SignalStatus
fake_mark_read(SignalCore *core, uint64_t request_id, const char *recipient,
               uint64_t timestamp)
{
    FakeReadMarker *marker = (FakeReadMarker *)core;

    marker->calls++;
    marker->request_id = request_id;
    marker->timestamp = timestamp;
    g_free(marker->recipient);
    marker->recipient = g_strdup(recipient);
    return marker->status;
}

static guint group_echo_writes;
static PurpleConversation *group_echo_conversation;
static char *group_echo_who;
static char *group_echo_message;
static PurpleMessageFlags group_echo_flags;
static time_t group_echo_timestamp;

static SignalStatus
fake_send_group_message(SignalCore *core, uint64_t request_id,
                        const char *group_key, const char *message)
{
    FakeGroupSender *sender = (FakeGroupSender *)core;

    sender->calls++;
    sender->request_id = request_id;
    g_free(sender->group_key);
    sender->group_key = g_strdup(group_key);
    g_free(sender->message);
    sender->message = g_strdup(message);
    return sender->status;
}

static void
capture_group_echo(PurpleConversation *conversation, const char *who,
                   const char *message, PurpleMessageFlags flags,
                   time_t timestamp)
{
    group_echo_writes++;
    group_echo_conversation = conversation;
    g_free(group_echo_who);
    group_echo_who = g_strdup(who);
    g_free(group_echo_message);
    group_echo_message = g_strdup(message);
    group_echo_flags = flags;
    group_echo_timestamp = timestamp;
}

static PurpleConversationUiOps group_echo_conversation_ops = {
    .write_chat = capture_group_echo,
};

typedef struct {
    guint writes;
    PurpleConversation *conversation;
    PurpleStoredImage *image;
    char *sender;
    PurpleMessageFlags flags;
    time_t timestamp;
} InlineMediaCapture;

static InlineMediaCapture inline_media_capture;

static void
capture_inline_media(PurpleConversation *conversation, const char *sender,
                     const char *message, PurpleMessageFlags flags,
                     time_t timestamp)
{
    PurpleStoredImage *image;
    int image_id = 0;

    g_assert_cmpint(sscanf(message, "<img id=\"%d\">", &image_id), ==, 1);
    image = purple_imgstore_find_by_id(image_id);
    g_assert_nonnull(image);
    g_assert_null(inline_media_capture.image);

    inline_media_capture.writes++;
    inline_media_capture.conversation = conversation;
    inline_media_capture.image = purple_imgstore_ref(image);
    inline_media_capture.sender = g_strdup(sender);
    inline_media_capture.flags = flags;
    inline_media_capture.timestamp = timestamp;
}

static PurpleConversationUiOps inline_media_conversation_ops = {
    .write_chat = capture_inline_media,
    .write_im = capture_inline_media,
    .has_focus = read_has_focus,
};

static guint presented_message_writes;

static void
capture_presented_message(PurpleConversation *conversation, const char *sender,
                          const char *message, PurpleMessageFlags flags,
                          time_t timestamp)
{
    (void)conversation;
    (void)sender;
    (void)message;
    (void)flags;
    (void)timestamp;
    presented_message_writes++;
}

static PurpleConversationUiOps presented_message_conversation_ops = {
    .write_chat = capture_presented_message,
    .write_im = capture_presented_message,
    .has_focus = read_has_focus,
};

static void
reset_inline_media_capture(void)
{
    g_clear_pointer(&inline_media_capture.image, purple_imgstore_unref);
    g_clear_pointer(&inline_media_capture.sender, g_free);
    inline_media_capture = (InlineMediaCapture){ 0 };
}

static void
reset_group_echo_capture(void)
{
    group_echo_writes = 0;
    group_echo_conversation = NULL;
    g_clear_pointer(&group_echo_who, g_free);
    g_clear_pointer(&group_echo_message, g_free);
    group_echo_flags = 0;
    group_echo_timestamp = 0;
}

static void
remove_tree(const char *path)
{
    GStatBuf status;

    if (g_lstat(path, &status) != 0)
        return;
    if (!S_ISDIR(status.st_mode)) {
        g_assert_cmpint(g_remove(path), ==, 0);
        return;
    }

    GDir *directory = g_dir_open(path, 0, NULL);
    g_assert_nonnull(directory);
    for (const char *name = g_dir_read_name(directory); name != NULL;
         name = g_dir_read_name(directory)) {
        g_autofree char *child = g_build_filename(path, name, NULL);
        remove_tree(child);
    }
    g_dir_close(directory);
    g_assert_cmpint(g_rmdir(path), ==, 0);
}

static PurpleChat *
new_group_chat(PurpleAccount *account, const char *group_id)
{
    GHashTable *components = g_hash_table_new_full(
        g_str_hash, g_str_equal, g_free, g_free);
    PurpleChat *chat;

    g_hash_table_insert(components, g_strdup(SIGNAL_GROUP_COMPONENT_KEY),
                        g_strdup(group_id));
    chat = purple_chat_new(account, "Test group", components);
    return chat;
}

static PurpleChat *
add_group_chat(PurpleAccount *account, PurpleGroup *group,
               const char *group_id, gboolean managed)
{
    PurpleChat *chat = new_group_chat(account, group_id);

    purple_blist_add_chat(chat, group, NULL);
    if (managed)
        purple_blist_node_set_bool(PURPLE_BLIST_NODE(chat),
                                   SIGNAL_SYNCED_GROUP_KEY, TRUE);
    return chat;
}

static PurpleBuddy *
add_buddy(PurpleAccount *account, PurpleGroup *group, const char *name,
          gboolean managed)
{
    PurpleBuddy *buddy = purple_buddy_new(account, name, NULL);

    purple_blist_add_buddy(buddy, NULL, group, NULL);
    if (managed)
        purple_blist_node_set_bool(PURPLE_BLIST_NODE(buddy),
                                   SIGNAL_SYNCED_BUDDY_KEY, TRUE);
    return buddy;
}

static void
test_default_blist_placement(PurpleAccount *account,
                             PurpleGroup **default_buddy_group,
                             PurpleGroup **default_chat_group)
{
    PurpleBuddy *control_buddy;
    PurpleBuddy *managed_buddy;
    PurpleChat *control_chat;
    PurpleChat *managed_chat;

    control_buddy = purple_buddy_new(account, "default-control", NULL);
    purple_blist_add_buddy(control_buddy, NULL, NULL, NULL);
    *default_buddy_group = purple_buddy_get_group(control_buddy);

    managed_buddy = purple_buddy_new(account, "default-managed", NULL);
    signal_blist_sync_add_buddy(managed_buddy);
    g_assert_true(purple_buddy_get_group(managed_buddy) ==
                  *default_buddy_group);
    g_assert_true(purple_blist_node_get_bool(PURPLE_BLIST_NODE(managed_buddy),
                                             SIGNAL_SYNCED_BUDDY_KEY));

    control_chat = new_group_chat(account, "default-chat-control");
    purple_blist_add_chat(control_chat, NULL, NULL);
    *default_chat_group = purple_chat_get_group(control_chat);

    managed_chat = new_group_chat(account, "default-chat-managed");
    signal_blist_sync_add_chat(managed_chat);
    g_assert_true(purple_chat_get_group(managed_chat) == *default_chat_group);
    g_assert_true(purple_blist_node_get_bool(PURPLE_BLIST_NODE(managed_chat),
                                             SIGNAL_SYNCED_GROUP_KEY));
}

static void
test_buddy_legacy_migration(PurpleAccount *account,
                            PurpleGroup *default_group)
{
    PurpleGroup *legacy_group;
    PurpleGroup *custom_group;
    PurpleBuddy *buddy;
    PurpleBuddy *merged_buddy;
    PurpleBuddy *migrated;
    PurpleBuddy *unmanaged;
    PurpleBuddy *destination_duplicate;
    PurpleContact *contact;

    legacy_group = purple_group_new(SIGNAL_LEGACY_BUDDY_GROUP);
    purple_blist_add_group(legacy_group, NULL);
    buddy = add_buddy(account, legacy_group, "legacy-managed", TRUE);
    contact = purple_buddy_get_contact(buddy);
    purple_blist_alias_contact(contact, "Merged local contact");
    merged_buddy = purple_buddy_new(account, "locally-merged", NULL);
    purple_blist_add_buddy(merged_buddy, contact, NULL, NULL);

    migrated = signal_blist_sync_migrate_buddy(buddy);
    g_assert_true(migrated == buddy);
    g_assert_true(purple_buddy_get_group(migrated) == default_group);
    g_assert_true(purple_buddy_get_contact(migrated) == contact);
    g_assert_true(purple_buddy_get_contact(merged_buddy) == contact);
    g_assert_true(purple_buddy_get_group(merged_buddy) == default_group);
    g_assert_cmpstr(purple_contact_get_alias(contact), ==,
                    "Merged local contact");
    g_assert_true(purple_blist_node_get_bool(PURPLE_BLIST_NODE(migrated),
                                             SIGNAL_SYNCED_BUDDY_KEY));
    g_assert_null(purple_find_group(SIGNAL_LEGACY_BUDDY_GROUP));

    legacy_group = purple_group_new(SIGNAL_LEGACY_BUDDY_GROUP);
    purple_blist_add_group(legacy_group, NULL);
    custom_group = purple_group_new("Signal managed custom placement");
    purple_blist_add_group(custom_group, NULL);
    buddy = add_buddy(account, custom_group, "custom-managed", TRUE);
    migrated = signal_blist_sync_migrate_buddy(buddy);
    g_assert_true(migrated == buddy);
    g_assert_true(purple_buddy_get_group(migrated) == custom_group);
    g_assert_true(purple_blist_node_get_bool(PURPLE_BLIST_NODE(migrated),
                                             SIGNAL_SYNCED_BUDDY_KEY));
    g_assert_true(purple_find_group(SIGNAL_LEGACY_BUDDY_GROUP) ==
                  legacy_group);
    purple_blist_remove_group(legacy_group);

    legacy_group = purple_group_new(SIGNAL_LEGACY_BUDDY_GROUP);
    purple_blist_add_group(legacy_group, NULL);
    unmanaged = add_buddy(account, legacy_group, "legacy-unmanaged", FALSE);
    migrated = signal_blist_sync_migrate_buddy(unmanaged);
    g_assert_true(migrated == unmanaged);
    g_assert_true(purple_buddy_get_group(unmanaged) == legacy_group);
    g_assert_false(purple_blist_node_get_bool(PURPLE_BLIST_NODE(unmanaged),
                                              SIGNAL_SYNCED_BUDDY_KEY));

    buddy = add_buddy(account, legacy_group, "legacy-managed-retained", TRUE);
    migrated = signal_blist_sync_migrate_buddy(buddy);
    g_assert_true(migrated == buddy);
    g_assert_true(purple_buddy_get_group(migrated) == default_group);
    g_assert_true(purple_find_group(SIGNAL_LEGACY_BUDDY_GROUP) ==
                  legacy_group);
    g_assert_nonnull(
        purple_blist_node_get_first_child(PURPLE_BLIST_NODE(legacy_group)));

    destination_duplicate =
        add_buddy(account, default_group, "legacy-deduplicate", FALSE);
    buddy = add_buddy(account, legacy_group, "legacy-deduplicate", TRUE);
    g_assert_true(signal_blist_sync_find_buddy(account,
                                               "legacy-deduplicate") == buddy);
    migrated = signal_blist_sync_migrate_buddy(buddy);
    g_assert_true(migrated == destination_duplicate);
    g_assert_true(purple_buddy_get_group(migrated) == default_group);
    g_assert_false(purple_blist_node_get_bool(PURPLE_BLIST_NODE(migrated),
                                              SIGNAL_SYNCED_BUDDY_KEY));

    purple_blist_remove_buddy(unmanaged);
    signal_blist_sync_remove_empty_legacy_buddy_group();
    g_assert_null(purple_find_group(SIGNAL_LEGACY_BUDDY_GROUP));
}

static void
test_buddy_legacy_adoption(PurplePluginProtocolInfo *protocol,
                           PurpleAccount *account,
                           PurpleGroup *default_group)
{
    PurpleAccount *unrelated_account;
    PurpleGroup *legacy_group;
    PurpleGroup *custom_group;
    PurpleBuddy *legacy;
    PurpleBuddy *adopted;
    PurpleBuddy *custom;
    PurpleBuddy *managed_custom;
    PurpleBuddy *priority_legacy;
    PurpleBuddy *destination_duplicate;
    PurpleBuddy *deduplicated;
    PurpleBuddy *merged_unrelated;
    PurpleBuddy *unrelated;
    PurpleContact *contact;

    unrelated_account =
        purple_account_new("unrelated-test", "prpl-unrelated-test");
    unrelated_account->presence =
        purple_presence_new_for_account(unrelated_account);
    purple_account_set_status_types(
        unrelated_account, protocol->status_types(unrelated_account));
    legacy_group = purple_group_new(SIGNAL_LEGACY_BUDDY_GROUP);
    purple_blist_add_group(legacy_group, NULL);
    custom_group = purple_group_new("Legacy adoption custom placement");
    purple_blist_add_group(custom_group, NULL);

    custom = add_buddy(account, custom_group, "custom-authoritative", FALSE);
    purple_blist_alias_buddy(custom, "Custom local alias");
    g_assert_true(signal_blist_sync_adopt_legacy_buddy(
                      account, "custom-authoritative") == custom);
    g_assert_true(purple_buddy_get_group(custom) == custom_group);
    g_assert_cmpstr(purple_buddy_get_local_buddy_alias(custom), ==,
                    "Custom local alias");
    g_assert_false(purple_blist_node_get_bool(PURPLE_BLIST_NODE(custom),
                                              SIGNAL_SYNCED_BUDDY_KEY));

    unrelated = add_buddy(unrelated_account, legacy_group,
                          "legacy-authoritative", FALSE);
    legacy = add_buddy(account, legacy_group, "legacy-authoritative", FALSE);
    purple_blist_alias_buddy(legacy, "Legacy local alias");
    contact = purple_buddy_get_contact(legacy);
    purple_blist_alias_contact(contact, "Merged legacy alias");
    merged_unrelated =
        purple_buddy_new(unrelated_account, "merged-unrelated", NULL);
    purple_blist_add_buddy(merged_unrelated, contact, NULL, NULL);

    adopted = signal_blist_sync_adopt_legacy_buddy(
        account, "legacy-authoritative");
    g_assert_nonnull(adopted);
    g_assert_true(purple_buddy_get_account(adopted) == account);
    g_assert_true(purple_buddy_get_group(adopted) == default_group);
    g_assert_cmpstr(purple_buddy_get_local_buddy_alias(adopted), ==,
                    "Legacy local alias");
    g_assert_true(purple_buddy_get_contact(adopted) == contact);
    g_assert_cmpstr(purple_contact_get_alias(contact), ==,
                    "Merged legacy alias");
    g_assert_true(purple_buddy_get_contact(merged_unrelated) == contact);
    g_assert_true(purple_buddy_get_group(merged_unrelated) == default_group);
    g_assert_false(purple_blist_node_get_bool(
        PURPLE_BLIST_NODE(merged_unrelated), SIGNAL_SYNCED_BUDDY_KEY));
    g_assert_true(purple_blist_node_get_bool(PURPLE_BLIST_NODE(adopted),
                                             SIGNAL_SYNCED_BUDDY_KEY));
    g_assert_true(purple_buddy_get_group(unrelated) == legacy_group);
    g_assert_false(purple_blist_node_get_bool(PURPLE_BLIST_NODE(unrelated),
                                              SIGNAL_SYNCED_BUDDY_KEY));

    managed_custom = add_buddy(account, custom_group,
                               "managed-custom-priority", TRUE);
    purple_blist_alias_buddy(managed_custom, "Managed custom alias");
    priority_legacy = add_buddy(account, legacy_group,
                                "managed-custom-priority", FALSE);
    g_assert_true(signal_blist_sync_find_buddy(
                      account, "managed-custom-priority") == managed_custom);
    g_assert_true(signal_blist_sync_adopt_legacy_buddy(
                      account, "managed-custom-priority") == managed_custom);
    g_assert_true(purple_buddy_get_group(managed_custom) == custom_group);
    g_assert_cmpstr(purple_buddy_get_local_buddy_alias(managed_custom), ==,
                    "Managed custom alias");
    g_assert_true(purple_buddy_get_group(priority_legacy) == legacy_group);
    g_assert_false(purple_blist_node_get_bool(
        PURPLE_BLIST_NODE(priority_legacy), SIGNAL_SYNCED_BUDDY_KEY));

    destination_duplicate = add_buddy(
        account, default_group, "legacy-adoption-deduplicate", FALSE);
    add_buddy(account, legacy_group, "legacy-adoption-deduplicate", FALSE);
    deduplicated = signal_blist_sync_adopt_legacy_buddy(
        account, "legacy-adoption-deduplicate");
    g_assert_true(deduplicated == destination_duplicate);
    g_assert_true(purple_buddy_get_group(deduplicated) == default_group);
    g_assert_false(purple_blist_node_get_bool(PURPLE_BLIST_NODE(deduplicated),
                                              SIGNAL_SYNCED_BUDDY_KEY));
    g_assert_null(purple_find_buddy_in_group(
        account, "legacy-adoption-deduplicate", legacy_group));

    purple_blist_remove_buddy(priority_legacy);
    purple_blist_remove_buddy(unrelated);
    purple_blist_remove_buddy(merged_unrelated);
    purple_blist_remove_buddy(adopted);
    purple_blist_remove_buddy(deduplicated);
    purple_blist_remove_buddy(managed_custom);
    signal_blist_sync_remove_empty_legacy_buddy_group();
    g_assert_null(purple_find_group(SIGNAL_LEGACY_BUDDY_GROUP));
    purple_blist_remove_buddy(custom);
    purple_blist_remove_group(custom_group);
    purple_account_destroy(unrelated_account);
}

static void
test_chat_legacy_migration(PurpleAccount *account,
                           PurpleGroup *default_group)
{
    PurpleGroup *legacy_group;
    PurpleGroup *custom_group;
    PurpleChat *chat;
    PurpleChat *selected;
    PurpleChat *unmanaged;
    PurpleChat *user_chat;
    guint removed = 0;

    legacy_group = purple_group_new(SIGNAL_LEGACY_CHAT_GROUP);
    purple_blist_add_group(legacy_group, NULL);
    chat = add_group_chat(account, legacy_group, "legacy-chat-managed", TRUE);
    signal_blist_sync_migrate_chat(chat);
    g_assert_true(purple_chat_get_group(chat) == default_group);
    g_assert_true(purple_blist_node_get_bool(PURPLE_BLIST_NODE(chat),
                                             SIGNAL_SYNCED_GROUP_KEY));
    g_assert_cmpstr(purple_chat_get_name(chat), ==, "Test group");
    g_assert_cmpstr(g_hash_table_lookup(purple_chat_get_components(chat),
                                        SIGNAL_GROUP_COMPONENT_KEY),
                    ==, "legacy-chat-managed");
    g_assert_null(purple_find_group(SIGNAL_LEGACY_CHAT_GROUP));

    legacy_group = purple_group_new(SIGNAL_LEGACY_CHAT_GROUP);
    purple_blist_add_group(legacy_group, NULL);
    custom_group = purple_group_new("Signal chat custom placement");
    purple_blist_add_group(custom_group, NULL);
    chat = add_group_chat(account, custom_group, "custom-chat-managed", TRUE);
    signal_blist_sync_migrate_chat(chat);
    g_assert_true(purple_chat_get_group(chat) == custom_group);
    g_assert_true(purple_blist_node_get_bool(PURPLE_BLIST_NODE(chat),
                                             SIGNAL_SYNCED_GROUP_KEY));
    g_assert_true(purple_find_group(SIGNAL_LEGACY_CHAT_GROUP) ==
                  legacy_group);
    purple_blist_remove_group(legacy_group);

    legacy_group = purple_group_new(SIGNAL_LEGACY_CHAT_GROUP);
    purple_blist_add_group(legacy_group, NULL);
    unmanaged =
        add_group_chat(account, legacy_group, "legacy-chat-unmanaged", FALSE);
    signal_blist_sync_migrate_chat(unmanaged);
    g_assert_true(purple_chat_get_group(unmanaged) == legacy_group);
    g_assert_false(purple_blist_node_get_bool(PURPLE_BLIST_NODE(unmanaged),
                                              SIGNAL_SYNCED_GROUP_KEY));

    chat =
        add_group_chat(account, legacy_group, "legacy-chat-retained", TRUE);
    signal_blist_sync_migrate_chat(chat);
    g_assert_true(purple_chat_get_group(chat) == default_group);
    g_assert_true(purple_find_group(SIGNAL_LEGACY_CHAT_GROUP) == legacy_group);
    g_assert_nonnull(
        purple_blist_node_get_first_child(PURPLE_BLIST_NODE(legacy_group)));

    purple_blist_remove_chat(unmanaged);
    signal_blist_sync_remove_empty_legacy_chat_group();
    g_assert_null(purple_find_group(SIGNAL_LEGACY_CHAT_GROUP));

    legacy_group = purple_group_new(SIGNAL_LEGACY_CHAT_GROUP);
    purple_blist_add_group(legacy_group, NULL);
    user_chat = add_group_chat(account, default_group,
                               "legacy-chat-duplicate", FALSE);
    add_group_chat(account, legacy_group, "legacy-chat-duplicate", TRUE);
    selected = signal_group_sync_find_chat(account, "legacy-chat-duplicate",
                                           &removed);
    g_assert_true(selected == user_chat);
    g_assert_cmpuint(removed, ==, 1);
    g_assert_null(purple_find_group(SIGNAL_LEGACY_CHAT_GROUP));
}

static void
test_group_conversation_identity(PurplePlugin *plugin,
                                 PurplePluginProtocolInfo *protocol,
                                 PurpleAccount *account, PurpleGroup *group)
{
    PurpleConnection gc = {
        .prpl = plugin,
        .state = PURPLE_CONNECTED,
        .account = account,
    };
    PurpleConnection reconnected_gc = {
        .prpl = plugin,
        .state = PURPLE_CONNECTED,
        .account = account,
    };
    SignalConnection connection = {
        .gc = &gc,
        .group_ids_by_key = g_hash_table_new_full(
            g_str_hash, g_str_equal, g_free, NULL),
        .group_keys_by_id = g_hash_table_new_full(
            g_direct_hash, g_direct_equal, NULL, g_free),
        .group_titles_by_key = g_hash_table_new_full(
            g_str_hash, g_str_equal, g_free, g_free),
        .group_members_by_key = g_hash_table_new_full(
            g_str_hash, g_str_equal, g_free,
            (GDestroyNotify)g_ptr_array_unref),
        .active_group_keys = g_hash_table_new_full(
            g_str_hash, g_str_equal, g_free, NULL),
        .pending_group_joins = g_hash_table_new_full(
            g_str_hash, g_str_equal, g_free, NULL),
        .pending_group_leaves = g_hash_table_new_full(
            g_str_hash, g_str_equal, g_free, NULL),
        .group_leave_requests = g_ptr_array_new(),
        .local_aci =
            g_strdup("11111111-1111-4111-8111-111111111111"),
        .remote_profile_name = g_strdup("Remote Signal profile"),
        .next_group_id = 1,
        .group_snapshot_complete = TRUE,
    };
    PurpleChat *first;
    PurpleChat *legacy;
    PurpleChat *second;
    PurpleConversation *first_conversation;
    PurpleConversation *legacy_conversation;
    PurpleConversation *second_conversation;
    GHashTable *legacy_components;
    FakeGroupSender group_sender = {
        .status = SIGNAL_STATUS_OK,
    };
    union {
        gpointer pointer;
        SignalHandleEventFunc function;
    } handle_event = { 0 };
    guint reconnect_joined = 0;

    purple_account_set_connection(account, &gc);
    purple_connection_set_protocol_data(&gc, &connection);
    signal_contact_sync_init(&connection.group_sync);
    g_assert_true(g_module_symbol((GModule *)plugin->handle,
                                  "signal_handle_event",
                                  &handle_event.pointer));

    first = add_group_chat(account, group, "stable-conversation-one", TRUE);
    second = add_group_chat(account, group, "stable-conversation-two", FALSE);
    legacy_components = g_hash_table_new_full(g_str_hash, g_str_equal, g_free,
                                              g_free);
    g_hash_table_insert(legacy_components,
                        g_strdup(SIGNAL_LEGACY_GROUP_COMPONENT_KEY),
                        g_strdup("stable-conversation-legacy"));
    legacy = purple_chat_new(account, "Legacy local title", legacy_components);
    purple_blist_add_chat(legacy, group, NULL);
    purple_blist_alias_chat(first, "Shared Signal title");
    purple_blist_alias_chat(second, "Shared Signal title");
    g_hash_table_insert(connection.group_titles_by_key,
                        g_strdup("stable-conversation-one"),
                        g_strdup("Remote title one"));
    g_hash_table_insert(connection.group_titles_by_key,
                        g_strdup("stable-conversation-two"),
                        g_strdup("Remote title two"));
    g_hash_table_insert(connection.group_titles_by_key,
                        g_strdup("stable-conversation-legacy"),
                        g_strdup("Remote legacy title"));
    g_hash_table_add(connection.active_group_keys,
                     g_strdup("stable-conversation-one"));
    g_hash_table_add(connection.active_group_keys,
                     g_strdup("stable-conversation-two"));
    g_hash_table_add(connection.active_group_keys,
                     g_strdup("stable-conversation-legacy"));

    g_assert_true(protocol->find_blist_chat(account,
                                            "stable-conversation-one") ==
                  first);
    g_assert_true(protocol->find_blist_chat(account,
                                            "stable-conversation-two") ==
                  second);
    g_assert_true(protocol->find_blist_chat(account,
                                            "stable-conversation-legacy") ==
                  legacy);
    g_assert_true(purple_blist_find_chat(account,
                                         "stable-conversation-one") == first);
    g_assert_true(purple_blist_find_chat(account,
                                         "stable-conversation-two") == second);

    protocol->join_chat(&gc, purple_chat_get_components(first));
    protocol->join_chat(&gc, purple_chat_get_components(second));
    protocol->join_chat(&gc, purple_chat_get_components(legacy));
    first_conversation = purple_find_chat(&gc, 1);
    second_conversation = purple_find_chat(&gc, 2);
    legacy_conversation = purple_find_chat(&gc, 3);
    g_assert_nonnull(first_conversation);
    g_assert_nonnull(second_conversation);
    g_assert_nonnull(legacy_conversation);
    g_assert_true(first_conversation != second_conversation);
    g_assert_cmpstr(purple_conversation_get_name(first_conversation), ==,
                    "stable-conversation-one");
    g_assert_cmpstr(purple_conversation_get_name(second_conversation), ==,
                    "stable-conversation-two");
    g_assert_cmpstr(purple_conversation_get_name(legacy_conversation), ==,
                    "stable-conversation-legacy");
    g_assert_cmpstr(purple_conversation_get_title(first_conversation), ==,
                    "Shared Signal title");
    g_assert_cmpstr(purple_conversation_get_title(second_conversation), ==,
                    "Shared Signal title");
    g_assert_cmpstr(purple_conversation_get_title(legacy_conversation), ==,
                    "Legacy local title");

    purple_account_set_alias(account, "Local Purple alias");
    g_assert_cmpstr(
        purple_conv_chat_get_nick(PURPLE_CONV_CHAT(first_conversation)), ==,
        connection.local_aci);
    purple_conv_chat_add_user(PURPLE_CONV_CHAT(first_conversation),
                              connection.local_aci, NULL,
                              PURPLE_CBFLAGS_NONE, FALSE);
    PurpleConvChatBuddy *local_member = purple_conv_chat_cb_find(
        PURPLE_CONV_CHAT(first_conversation), connection.local_aci);
    g_assert_nonnull(local_member);
    g_assert_cmpstr(local_member->name, ==, connection.local_aci);
    g_assert_cmpstr(local_member->alias, ==, "Local Purple alias");

    purple_account_set_alias(account, NULL);
    protocol->join_chat(&gc, purple_chat_get_components(first));
    purple_conv_chat_add_user(PURPLE_CONV_CHAT(first_conversation),
                              connection.local_aci, NULL,
                              PURPLE_CBFLAGS_NONE, FALSE);
    local_member = purple_conv_chat_cb_find(
        PURPLE_CONV_CHAT(first_conversation), connection.local_aci);
    g_assert_nonnull(local_member);
    g_assert_cmpstr(local_member->name, ==, connection.local_aci);
    g_assert_cmpstr(local_member->alias, ==, "Remote Signal profile");
    g_clear_pointer(&connection.remote_profile_name, g_free);
    g_assert_null(protocol->get_cb_alias(&gc, 1, connection.local_aci));

    /* Pidgin autosets the title after member updates. While the account is
     * reconnecting, Purple cannot look up the saved chat and uses its opaque
     * canonical name. A member refresh must restore the local group title. */
    purple_blist_alias_chat(first, "Reconnect display title");
    purple_conv_chat_add_user(PURPLE_CONV_CHAT(first_conversation),
                              "existing-member", NULL,
                              PURPLE_CBFLAGS_NONE, FALSE);
    purple_conversation_set_ui_ops(first_conversation,
                                   &reconnect_conversation_ops);
    member_update_title_autosets = 0;
    gc.state = PURPLE_CONNECTING;
    protocol->join_chat(&gc, purple_chat_get_components(first));
    g_assert_cmpuint(member_update_title_autosets, ==, 1);
    g_assert_cmpint(purple_conversation_get_type(first_conversation), ==,
                    PURPLE_CONV_TYPE_CHAT);
    g_assert_cmpstr(purple_conversation_get_name(first_conversation), ==,
                    "stable-conversation-one");
    g_assert_cmpstr(purple_conversation_get_title(first_conversation), ==,
                    "Reconnect display title");
    g_assert_null(purple_find_conversation_with_account(
        PURPLE_CONV_TYPE_IM, "stable-conversation-one", account));
    purple_conversation_set_ui_ops(first_conversation, NULL);
    gc.state = PURPLE_CONNECTED;

    GList *menu = protocol->blist_node_menu(PURPLE_BLIST_NODE(first));
    g_assert_cmpuint(g_list_length(menu), ==, 1);
    g_assert_cmpstr(((PurpleMenuAction *)menu->data)->label, ==,
                    "Leave Signal group…");
    g_list_free_full(menu, (GDestroyNotify)purple_menu_action_free);
    g_assert_null(protocol->blist_node_menu(PURPLE_BLIST_NODE(second)));

    g_hash_table_add(connection.pending_group_leaves,
                     g_strdup("stable-conversation-one"));
    g_assert_null(protocol->blist_node_menu(PURPLE_BLIST_NODE(first)));
    g_assert_cmpint(protocol->chat_send(&gc, 1, "leaving", 0), ==, -ENOENT);
    g_assert_false(protocol->chat_can_receive_file(&gc, 1));
    g_hash_table_remove(connection.pending_group_leaves,
                        "stable-conversation-one");
    g_hash_table_remove(connection.active_group_keys,
                        "stable-conversation-one");
    g_assert_null(protocol->blist_node_menu(PURPLE_BLIST_NODE(first)));
    g_assert_cmpint(protocol->chat_send(&gc, 1, "blocked", 0), ==, -ENOENT);
    g_assert_false(protocol->chat_can_receive_file(&gc, 1));
    g_hash_table_add(connection.active_group_keys,
                     g_strdup("stable-conversation-one"));
    g_assert_true(protocol->chat_can_receive_file(&gc, 1));

    reset_group_echo_capture();
    connection.core = (SignalCore *)&group_sender;
    connection.send_group_message = fake_send_group_message;
    connection.next_request_id = 41;
    purple_conv_chat_set_nick(PURPLE_CONV_CHAT(first_conversation),
                              "local-group-member");
    purple_conversation_set_ui_ops(first_conversation,
                                   &group_echo_conversation_ops);
    purple_conv_chat_send_with_flags(PURPLE_CONV_CHAT(first_conversation),
                                     "Hello <b>group</b>",
                                     PURPLE_MESSAGE_NO_LOG);
    g_assert_cmpuint(group_sender.calls, ==, 1);
    g_assert_cmpuint(group_sender.request_id, ==, 41);
    g_assert_cmpstr(group_sender.group_key, ==, "stable-conversation-one");
    g_assert_cmpstr(group_sender.message, ==, "Hello group");
    g_assert_cmpuint(group_echo_writes, ==, 1);
    g_assert_true(group_echo_conversation == first_conversation);
    g_assert_cmpstr(group_echo_who, ==, "local-group-member");
    g_assert_cmpstr(group_echo_message, ==, "Hello <b>group</b>");
    g_assert_true((group_echo_flags & PURPLE_MESSAGE_SEND) != 0);
    g_assert_true((group_echo_flags & PURPLE_MESSAGE_NO_LOG) != 0);
    g_assert_true((group_echo_flags & PURPLE_MESSAGE_RECV) == 0);
    g_assert_true((group_echo_flags & PURPLE_MESSAGE_REMOTE_SEND) == 0);
    g_assert_cmpint(group_echo_timestamp, >, 0);
    g_assert_cmpuint(connection.next_request_id, ==, 42);

    purple_conv_chat_send_with_flags(PURPLE_CONV_CHAT(first_conversation),
                                     "hidden group message",
                                     PURPLE_MESSAGE_INVISIBLE);
    g_assert_cmpuint(group_sender.calls, ==, 2);
    g_assert_cmpstr(group_sender.message, ==, "hidden group message");
    g_assert_cmpuint(group_echo_writes, ==, 1);

    group_sender.status = SIGNAL_STATUS_QUEUE_FULL;
    g_assert_cmpint(protocol->chat_send(
                        &gc, 1, "queue failure", PURPLE_MESSAGE_SEND),
                    ==, -EAGAIN);
    g_assert_cmpuint(group_sender.calls, ==, 3);
    g_assert_cmpuint(group_echo_writes, ==, 1);

    group_sender.status = SIGNAL_STATUS_NOT_READY;
    g_assert_cmpint(protocol->chat_send(
                        &gc, 1, "not ready", PURPLE_MESSAGE_SEND),
                    ==, -ENOTCONN);
    g_assert_cmpuint(group_sender.calls, ==, 4);
    g_assert_cmpuint(group_echo_writes, ==, 1);

    SignalEvent recovering = {
        .abi_version = SIGNAL_CORE_ABI_VERSION,
        .struct_size = sizeof(SignalEvent),
        .kind = SIGNAL_EVENT_RECOVERING,
    };
    g_assert_true(handle_event.function(&connection, &recovering));
    g_assert_false(connection.group_snapshot_complete);
    g_assert_null(protocol->blist_node_menu(PURPLE_BLIST_NODE(first)));
    g_assert_cmpint(protocol->chat_send(
                        &gc, 1, "blocked during recovery",
                        PURPLE_MESSAGE_SEND),
                    ==, -ENOENT);
    g_assert_false(protocol->chat_can_receive_file(&gc, 1));
    g_assert_cmpuint(group_sender.calls, ==, 4);
    connection.group_snapshot_complete = TRUE;

    purple_conversation_set_ui_ops(first_conversation, NULL);
    connection.core = NULL;
    connection.send_group_message = NULL;
    g_clear_pointer(&group_sender.group_key, g_free);
    g_clear_pointer(&group_sender.message, g_free);
    reset_group_echo_capture();

    purple_blist_alias_chat(first, "Local display title");
    protocol->join_chat(&gc, purple_chat_get_components(first));
    g_assert_true(purple_find_chat(&gc, 1) == first_conversation);
    g_assert_cmpstr(purple_conversation_get_title(first_conversation), ==,
                    "Local display title");
    g_assert_cmpstr(purple_conversation_get_title(second_conversation), ==,
                    "Shared Signal title");

    serv_got_chat_left(&gc, 1);
    serv_got_chat_left(&gc, 2);
    serv_got_chat_left(&gc, 3);
    g_assert_null(gc.buddy_chats);
    g_assert_true(purple_conv_chat_has_left(
        PURPLE_CONV_CHAT(first_conversation)));
    g_assert_true(purple_conv_chat_has_left(
        PURPLE_CONV_CHAT(second_conversation)));
    g_assert_true(purple_conv_chat_has_left(
        PURPLE_CONV_CHAT(legacy_conversation)));

    connection.gc = &reconnected_gc;
    connection.group_snapshot_complete = FALSE;
    /* Reuse the old second conversation's numeric ID. Stable group identity,
     * rather than a connection-local ID, must select the first conversation. */
    connection.next_group_id = 2;
    g_hash_table_remove_all(connection.group_ids_by_key);
    g_hash_table_remove_all(connection.group_keys_by_id);
    purple_account_set_connection(account, &reconnected_gc);
    purple_connection_set_protocol_data(&gc, NULL);
    purple_connection_set_protocol_data(&reconnected_gc, &connection);
    purple_signal_connect(purple_conversations_get_handle(), "chat-joined",
                          &reconnect_joined,
                          PURPLE_CALLBACK(count_chat_joined),
                          &reconnect_joined);

    protocol->join_chat(&reconnected_gc, purple_chat_get_components(first));
    protocol->join_chat(&reconnected_gc, purple_chat_get_components(first));
    g_assert_cmpuint(g_hash_table_size(connection.pending_group_joins), ==, 1);
    g_assert_true(purple_conv_chat_has_left(
        PURPLE_CONV_CHAT(first_conversation)));
    g_assert_true(signal_group_sync_defer_join(
        connection.pending_group_joins, FALSE, "inactive-after-sync"));

    member_update_title_autosets = 0;
    purple_conversation_set_ui_ops(first_conversation,
                                   &reconnect_conversation_ops);
    dispatch_group_snapshot(handle_event.function, &connection);

    g_assert_cmpuint(g_hash_table_size(connection.pending_group_joins), ==, 0);
    g_assert_true(connection.group_snapshot_complete);
    g_assert_false(purple_conv_chat_has_left(
        PURPLE_CONV_CHAT(first_conversation)));
    g_assert_true(signal_group_sync_lookup_conversation(
                      account, "stable-conversation-one") ==
                  first_conversation);
    g_assert_cmpint(purple_conv_chat_get_id(
                        PURPLE_CONV_CHAT(first_conversation)),
                    ==, 2);
    g_assert_true(purple_conv_chat_has_left(
        PURPLE_CONV_CHAT(second_conversation)));
    g_assert_true(purple_conv_chat_find_user(
        PURPLE_CONV_CHAT(first_conversation),
        "aci:11111111-1111-4111-8111-111111111111"));
    /* Rejoining clears the old roster once, then the authoritative refresh
     * replaces it. A pre-rejoin refresh of the left conversation adds a third
     * callback and regresses the guard this test covers. */
    g_assert_cmpuint(member_update_title_autosets, ==, 2);
    g_assert_cmpuint(g_slist_length(reconnected_gc.buddy_chats), ==, 1);
    g_assert_cmpuint(reconnect_joined, ==, 1);

    reset_group_echo_capture();
    group_sender.status = SIGNAL_STATUS_OK;
    group_sender.calls = 0;
    connection.core = (SignalCore *)&group_sender;
    connection.send_group_message = fake_send_group_message;
    connection.next_request_id = 51;
    purple_conversation_set_ui_ops(first_conversation,
                                   &group_echo_conversation_ops);
    purple_conv_chat_send_with_flags(PURPLE_CONV_CHAT(first_conversation),
                                     "Recovered group message",
                                     PURPLE_MESSAGE_NO_LOG);
    g_assert_cmpuint(group_sender.calls, ==, 1);
    g_assert_cmpstr(group_sender.group_key, ==, "stable-conversation-one");
    g_assert_cmpuint(group_echo_writes, ==, 1);
    g_assert_true(group_echo_conversation == first_conversation);

    purple_conversation_set_ui_ops(first_conversation, NULL);
    connection.core = NULL;
    connection.send_group_message = NULL;
    g_clear_pointer(&group_sender.group_key, g_free);
    g_clear_pointer(&group_sender.message, g_free);
    reset_group_echo_capture();

    dispatch_group_snapshot(handle_event.function, &connection);
    g_assert_cmpuint(g_slist_length(reconnected_gc.buddy_chats), ==, 1);
    g_assert_cmpint(purple_conv_chat_get_id(
                        PURPLE_CONV_CHAT(first_conversation)),
                    ==, 2);
    g_assert_cmpuint(reconnect_joined, ==, 1);
    purple_signals_disconnect_by_handle(&reconnect_joined);

    purple_conversation_destroy(legacy_conversation);
    purple_conversation_destroy(second_conversation);
    purple_conversation_destroy(first_conversation);
    g_assert_null(reconnected_gc.buddy_chats);
    purple_blist_remove_chat(legacy);
    purple_blist_remove_chat(second);
    purple_blist_remove_chat(first);
    purple_account_set_connection(account, NULL);
    purple_connection_set_protocol_data(&reconnected_gc, NULL);
    signal_contact_sync_clear(&connection.group_sync);
    g_ptr_array_unref(connection.group_leave_requests);
    g_hash_table_unref(connection.pending_group_leaves);
    g_hash_table_unref(connection.pending_group_joins);
    g_hash_table_unref(connection.active_group_keys);
    g_hash_table_unref(connection.group_members_by_key);
    g_hash_table_unref(connection.group_titles_by_key);
    g_hash_table_unref(connection.group_keys_by_id);
    g_hash_table_unref(connection.group_ids_by_key);
    g_free(connection.remote_profile_name);
    g_free(connection.local_aci);
}

static void
test_remove_managed_group_chats(PurpleAccount *account, PurpleGroup *group)
{
    PurpleChat *unmanaged = add_group_chat(account, group,
                                           "removed-managed-chats", FALSE);

    add_group_chat(account, group, "removed-managed-chats", TRUE);
    add_group_chat(account, group, "removed-managed-chats", TRUE);
    g_assert_cmpuint(signal_group_sync_remove_managed_chats(
                         account, "removed-managed-chats"),
                     ==, 2);
    g_assert_true(signal_group_sync_lookup_chat(account,
                                                "removed-managed-chats") ==
                  unmanaged);
    purple_blist_remove_chat(unmanaged);
}

static void
test_group_title_tracking(PurpleAccount *account, PurpleGroup *group)
{
    PurpleChat *chat;
    PurpleChat *upgraded;

    chat = add_group_chat(account, group, "tracked-title", TRUE);
    upgraded = add_group_chat(account, group, "upgraded-local-title", TRUE);
    purple_blist_alias_chat(chat, "Initial Signal title");
    signal_group_sync_update_title(chat, "tracked-title",
                                   "Initial Signal title");
    signal_group_sync_update_title(chat, "tracked-title",
                                   "Updated Signal title");
    g_assert_cmpstr(purple_chat_get_name(chat), ==, "Updated Signal title");

    purple_blist_alias_chat(chat, "Local display title");
    signal_group_sync_update_title(chat, "tracked-title",
                                   "Another Signal title");
    g_assert_cmpstr(purple_chat_get_name(chat), ==, "Local display title");
    g_assert_cmpstr(purple_blist_node_get_string(
                        PURPLE_BLIST_NODE(chat),
                        SIGNAL_SYNCED_GROUP_TITLE_KEY),
                    ==, "Another Signal title");

    /* In live Purple, a cleared alias resolves to the stable component.
     * Model that resolved display name directly so this regression exercises
     * signal-purple's title-following policy rather than Purple's fallback. */
    purple_blist_alias_chat(chat, "tracked-title");
    signal_group_sync_update_title(chat, "tracked-title",
                                   "Restored Signal title");
    g_assert_cmpstr(purple_chat_get_name(chat), ==, "Restored Signal title");

    purple_blist_alias_chat(upgraded, "Pre-existing local title");
    signal_group_sync_update_title(upgraded, "upgraded-local-title",
                                   "Current Signal title");
    g_assert_cmpstr(purple_chat_get_name(upgraded), ==,
                    "Pre-existing local title");

    purple_blist_remove_chat(upgraded);
    purple_blist_remove_chat(chat);
}

static SignalConnection *
new_transfer_connection(PurpleConnection *gc)
{
    SignalConnection *connection;

    g_assert_nonnull(connection_new);
    connection = connection_new(gc, "plugin-probe-store");
    g_assert_nonnull(connection);
    g_assert_cmpstr(connection->store_path, ==, "plugin-probe-store");
    g_assert_nonnull(connection->group_ids_by_key);
    g_assert_nonnull(connection->group_keys_by_id);
    g_assert_nonnull(connection->group_titles_by_key);
    g_assert_nonnull(connection->group_members_by_key);
    g_assert_nonnull(connection->active_group_keys);
    g_assert_nonnull(connection->pending_group_joins);
    g_assert_nonnull(connection->pending_group_leaves);
    g_assert_nonnull(connection->identity_changes_seen);
    g_assert_nonnull(connection->pending_identity_changes);
    g_assert_nonnull(connection->outgoing_attachments);
    g_assert_nonnull(connection->outgoing_attachment_contexts);
    g_assert_nonnull(connection->pending_reads);
    g_assert_nonnull(connection->group_leave_requests);
    g_assert_cmpuint(connection->next_group_id, ==, 1);
    g_assert_cmpuint(connection->next_request_id, ==, 1);
    purple_connection_set_protocol_data(gc, connection);
    return connection;
}

static gboolean
unused_source_callback(gpointer data)
{
    (void)data;
    return G_SOURCE_REMOVE;
}

static void
mark_source_finalized(gpointer data)
{
    gboolean *finalized = data;

    *finalized = TRUE;
}

static void
test_connection_owned_resource_cleanup(PurplePluginProtocolInfo *protocol)
{
    PurpleAccount *account =
        purple_account_new("connection-lifecycle", SIGNAL_PLUGIN_ID);
    PurpleConnection gc = {
        .state = PURPLE_CONNECTED,
        .account = account,
    };
    SignalConnection *connection;
    gboolean source_finalized = FALSE;
    GSource *source = g_idle_source_new();

    purple_account_set_connection(account, &gc);
    connection = new_transfer_connection(&gc);
    g_source_set_callback(source, unused_source_callback, &source_finalized,
                          mark_source_finalized);
    connection->poll_source = source;

    protocol->close(&gc);

    g_assert_null(purple_connection_get_protocol_data(&gc));
    g_assert_true(source_finalized);
    purple_account_set_connection(account, NULL);
    purple_account_destroy(account);
}

static void
test_standard_conversation_logging(PurplePlugin *plugin,
                                   PurplePluginProtocolInfo *protocol)
{
    const gboolean configured_states[] = { FALSE, TRUE };
    const gboolean original_im_logging =
        purple_prefs_get_bool("/purple/logging/log_ims");
    const gboolean original_chat_logging =
        purple_prefs_get_bool("/purple/logging/log_chats");
    union {
        gpointer pointer;
        SignalDispatchEventFunc function;
    } dispatch_event = { 0 };

    g_assert_true(g_module_symbol((GModule *)plugin->handle,
                                  "signal_dispatch_event",
                                  &dispatch_event.pointer));

    for (guint index = 0; index < G_N_ELEMENTS(configured_states); index++) {
        const gboolean configured = configured_states[index];
        g_autofree char *username =
            g_strdup_printf("conversation-logging-%u", index);
        PurpleAccount *account =
            purple_account_new(username, SIGNAL_PLUGIN_ID);
        PurpleConnection gc = {
            .prpl = plugin,
            .state = PURPLE_CONNECTED,
            .account = account,
        };
        SignalConnection *connection;
        PurpleConversation *direct;
        PurpleConversation *group;
        int group_id;
        SignalEvent direct_event = {
            .abi_version = SIGNAL_CORE_ABI_VERSION,
            .struct_size = sizeof(SignalEvent),
            .kind = SIGNAL_EVENT_MESSAGE,
            .peer_id = "aci:logging-contact",
            .text = "direct logging probe",
        };
        SignalEvent group_event = {
            .abi_version = SIGNAL_CORE_ABI_VERSION,
            .struct_size = sizeof(SignalEvent),
            .kind = SIGNAL_EVENT_GROUP_MESSAGE,
            .peer_id = "aci:logging-member",
            .chat_id = "logging-group",
            .title = "Logging group",
            .text = "group logging probe",
        };
        SignalEvent malformed_direct = direct_event;
        SignalEvent malformed_group = group_event;
        SignalEvent malformed_attachment = {
            .abi_version = SIGNAL_CORE_ABI_VERSION,
            .struct_size = sizeof(SignalEvent),
            .kind = SIGNAL_EVENT_ATTACHMENT,
            .request_id = 103,
            .peer_id = direct_event.peer_id,
            .data_len = 1,
        };
        gboolean accepted = TRUE;

        purple_prefs_set_bool("/purple/logging/log_ims", configured);
        purple_prefs_set_bool("/purple/logging/log_chats", configured);
        purple_account_set_connection(account, &gc);
        connection = new_transfer_connection(&gc);
        connection->group_snapshot_complete = TRUE;

        malformed_direct.request_id = 101;
        malformed_direct.peer_id = "";
        g_assert_true(dispatch_event.function(connection, &malformed_direct,
                                              &accepted));
        g_assert_false(accepted);
        malformed_group.request_id = 102;
        malformed_group.chat_id = "";
        accepted = TRUE;
        g_assert_true(dispatch_event.function(connection, &malformed_group,
                                              &accepted));
        g_assert_false(accepted);
        accepted = TRUE;
        g_assert_true(dispatch_event.function(
            connection, &malformed_attachment, &accepted));
        g_assert_false(accepted);
        g_assert_null(purple_find_conversation_with_account(
            PURPLE_CONV_TYPE_IM, direct_event.peer_id, account));
        g_assert_null(signal_group_sync_lookup_conversation(
            account, group_event.chat_id));
        g_assert_false(g_hash_table_contains(connection->active_group_keys,
                                             group_event.chat_id));

        g_assert_true(dispatch_event.function(connection, &direct_event,
                                              &accepted));
        g_assert_true(accepted);
        accepted = FALSE;
        g_assert_true(dispatch_event.function(connection, &group_event,
                                              &accepted));
        g_assert_true(accepted);
        direct = purple_find_conversation_with_account(
            PURPLE_CONV_TYPE_IM, direct_event.peer_id, account);
        group = signal_group_sync_lookup_conversation(account,
                                                      group_event.chat_id);
        g_assert_nonnull(direct);
        g_assert_nonnull(group);
        g_assert_cmpint(purple_conversation_is_logging(direct), ==,
                        configured);
        g_assert_cmpint(purple_conversation_is_logging(group), ==,
                        configured);

        purple_conversation_set_logging(direct, !configured);
        purple_conversation_set_logging(group, !configured);
        g_assert_true(dispatch_event.function(connection, &direct_event,
                                              &accepted));
        g_assert_true(accepted);
        accepted = FALSE;
        g_assert_true(dispatch_event.function(connection, &group_event,
                                              &accepted));
        g_assert_true(accepted);
        g_assert_cmpint(purple_conversation_is_logging(direct), ==,
                        !configured);
        g_assert_cmpint(purple_conversation_is_logging(group), ==,
                        !configured);

        group_id =
            purple_conv_chat_get_id(PURPLE_CONV_CHAT(group));
        g_assert_cmpint(protocol->send_im(&gc, direct_event.peer_id,
                                          "outbound direct probe", 0),
                        ==, -EINVAL);
        g_assert_cmpint(protocol->chat_send(&gc, group_id,
                                            "outbound group probe", 0),
                        ==, -EINVAL);
        g_assert_cmpint(purple_conversation_is_logging(direct), ==,
                        !configured);
        g_assert_cmpint(purple_conversation_is_logging(group), ==,
                        !configured);

        serv_got_chat_left(&gc, group_id);
        purple_conversation_destroy(group);
        purple_conversation_destroy(direct);
        protocol->close(&gc);
        purple_account_set_connection(account, NULL);
        purple_account_destroy(account);
    }

    purple_prefs_set_bool("/purple/logging/log_ims", original_im_logging);
    purple_prefs_set_bool("/purple/logging/log_chats",
                          original_chat_logging);
}

static void
test_pending_read_receipt_admission(PurplePlugin *plugin,
                                    PurplePluginProtocolInfo *protocol)
{
    union {
        gpointer pointer;
        SignalDispatchEventFunc function;
    } dispatch_event = { 0 };
    PurpleAccount *account =
        purple_account_new("pending-read-admission", SIGNAL_PLUGIN_ID);
    PurpleConnection gc = {
        .prpl = plugin,
        .state = PURPLE_CONNECTED,
        .account = account,
    };
    FakeReadMarker marker = {
        .status = SIGNAL_STATUS_OK,
    };
    SignalConnection *connection;
    PurpleConversation *conversation;
    SignalEvent message = {
        .abi_version = SIGNAL_CORE_ABI_VERSION,
        .struct_size = sizeof(SignalEvent),
        .kind = SIGNAL_EVENT_MESSAGE,
        .request_id = 1,
        .peer_id = "aci:pending-read-contact",
        .text = "pending read probe",
        .timestamp_ms = 100,
    };
    SignalEvent ready = {
        .abi_version = SIGNAL_CORE_ABI_VERSION,
        .struct_size = sizeof(SignalEvent),
        .kind = SIGNAL_EVENT_READY,
    };
    gboolean accepted = FALSE;
    guint retry_source;

    g_assert_true(g_module_symbol((GModule *)plugin->handle,
                                  "signal_dispatch_event",
                                  &dispatch_event.pointer));
    purple_account_set_connection(account, &gc);
    connection = new_transfer_connection(&gc);
    connection->core = (SignalCore *)&marker;
    connection->mark_read = fake_mark_read;

    read_conversation_focused = FALSE;
    g_assert_true(dispatch_event.function(connection, &message, &accepted));
    g_assert_true(accepted);
    g_assert_cmpuint(g_hash_table_size(connection->pending_reads), ==, 1);
    g_assert_cmpuint(marker.calls, ==, 0);
    g_assert_true(dispatch_event.function(connection, &message, &accepted));
    g_assert_cmpuint(g_hash_table_size(connection->pending_reads), ==, 1);

    conversation = purple_find_conversation_with_account(
        PURPLE_CONV_TYPE_IM, message.peer_id, account);
    g_assert_nonnull(conversation);
    purple_conversation_set_ui_ops(conversation, &read_conversation_ops);
    read_conversation_focused = TRUE;
    marker.status = SIGNAL_STATUS_QUEUE_FULL;
    message.timestamp_ms = 101;
    g_assert_true(dispatch_event.function(connection, &message, &accepted));
    g_assert_cmpuint(marker.calls, ==, 1);
    g_assert_cmpuint(g_hash_table_size(connection->pending_reads), ==, 2);
    g_assert_cmpuint(connection->pending_read_retry_id, !=, 0);

    marker.status = SIGNAL_STATUS_OK;
    gint64 deadline = g_get_monotonic_time() + G_TIME_SPAN_SECOND;
    while (g_hash_table_size(connection->pending_reads) != 0 &&
           g_get_monotonic_time() < deadline)
        g_main_context_iteration(NULL, FALSE);
    g_assert_cmpuint(g_hash_table_size(connection->pending_reads), ==, 0);
    g_assert_cmpuint(marker.calls, ==, 3);
    g_assert_cmpuint(connection->pending_read_retry_id, ==, 0);

    marker.status = SIGNAL_STATUS_NOT_READY;
    message.timestamp_ms = 102;
    g_assert_true(dispatch_event.function(connection, &message, &accepted));
    g_assert_cmpuint(g_hash_table_size(connection->pending_reads), ==, 1);
    g_assert_cmpuint(connection->pending_read_retry_id, ==, 0);
    guint calls_before_ready = marker.calls;
    marker.status = SIGNAL_STATUS_OK;
    g_assert_true(dispatch_event.function(connection, &ready, &accepted));
    g_assert_cmpuint(marker.calls, ==, calls_before_ready + 1);
    g_assert_cmpuint(g_hash_table_size(connection->pending_reads), ==, 0);

    read_conversation_focused = FALSE;
    message.timestamp_ms = 103;
    g_assert_true(dispatch_event.function(connection, &message, &accepted));
    guint calls_before_unfocused_ready = marker.calls;
    g_assert_true(dispatch_event.function(connection, &ready, &accepted));
    g_assert_cmpuint(marker.calls, ==, calls_before_unfocused_ready);
    g_assert_cmpuint(g_hash_table_size(connection->pending_reads), ==, 1);

    read_conversation_focused = TRUE;
    marker.status = SIGNAL_STATUS_QUEUE_FULL;
    message.timestamp_ms = 104;
    g_assert_true(dispatch_event.function(connection, &message, &accepted));
    retry_source = connection->pending_read_retry_id;
    g_assert_cmpuint(retry_source, !=, 0);
    connection->core = NULL;
    protocol->close(&gc);
    g_assert_null(g_main_context_find_source_by_id(NULL, retry_source));

    purple_conversation_set_ui_ops(conversation, NULL);
    purple_conversation_destroy(conversation);
    purple_account_set_connection(account, NULL);
    purple_account_destroy(account);
    g_clear_pointer(&marker.recipient, g_free);
    read_conversation_focused = FALSE;
}

static void
test_inline_attachment_routing(PurplePlugin *plugin,
                               PurplePluginProtocolInfo *protocol)
{
    gsize png_len = 0;
    g_autofree guchar *png = g_base64_decode(
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
        &png_len);
    union {
        gpointer pointer;
        SignalDispatchEventFunc function;
    } dispatch_event = { 0 };
    PurpleAccount *account =
        purple_account_new("inline-attachment-routing", SIGNAL_PLUGIN_ID);
    PurpleConnection gc = {
        .prpl = plugin,
        .state = PURPLE_CONNECTED,
        .account = account,
    };
    FakeReadMarker marker = {
        .status = SIGNAL_STATUS_OK,
    };
    SignalConnection *connection;
    PurpleConversation *direct;
    PurpleConversation *group;
    SignalEvent direct_caption = {
        .abi_version = SIGNAL_CORE_ABI_VERSION,
        .struct_size = sizeof(SignalEvent),
        .kind = SIGNAL_EVENT_MESSAGE,
        .timestamp_ms = 2000,
        .peer_id = "aci:inline-direct",
        .text = "direct caption",
    };
    SignalEvent direct_attachment = {
        .abi_version = SIGNAL_CORE_ABI_VERSION,
        .struct_size = sizeof(SignalEvent),
        .kind = SIGNAL_EVENT_ATTACHMENT,
        .request_id = 9001,
        .timestamp_ms = 2000,
        .peer_id = "aci:inline-direct",
        .title = "direct.png",
        .text = "image/png",
        .data = png,
        .data_len = png_len,
    };
    SignalEvent group_message = {
        .abi_version = SIGNAL_CORE_ABI_VERSION,
        .struct_size = sizeof(SignalEvent),
        .kind = SIGNAL_EVENT_GROUP_MESSAGE,
        .peer_id = "aci:inline-group-member",
        .chat_id = "inline-media-group",
        .title = "Inline media group",
        .text = "group setup",
    };
    SignalEvent group_message_filtered = {
        .abi_version = SIGNAL_CORE_ABI_VERSION,
        .struct_size = sizeof(SignalEvent),
        .kind = SIGNAL_EVENT_GROUP_MESSAGE,
        .request_id = 9004,
        .timestamp_ms = 3000,
        .peer_id = "aci:inline-group-member",
        .chat_id = "inline-media-group",
        .title = "Inline media group",
        .text = "ignored group message",
    };
    SignalEvent group_attachment = {
        .abi_version = SIGNAL_CORE_ABI_VERSION,
        .struct_size = sizeof(SignalEvent),
        .kind = SIGNAL_EVENT_ATTACHMENT,
        .request_id = 9005,
        .timestamp_ms = 3000,
        .peer_id = "aci:inline-group-member",
        .chat_id = "inline-media-group",
        .title = "group.png",
        .text = "image/png",
        .data = png,
        .data_len = png_len,
    };
    gboolean accepted = FALSE;
    guint transfer_count = g_list_length(purple_xfers_get_all());

    g_assert_cmpuint(png_len, ==, 68);
    g_assert_true(g_module_symbol((GModule *)plugin->handle,
                                  "signal_dispatch_event",
                                  &dispatch_event.pointer));
    purple_account_set_connection(account, &gc);
    connection = new_transfer_connection(&gc);
    connection->core = (SignalCore *)&marker;
    connection->mark_read = fake_mark_read;

    direct = purple_conversation_new(PURPLE_CONV_TYPE_IM, account,
                                     direct_attachment.peer_id);
    purple_conversation_set_ui_ops(direct,
                                   &presented_message_conversation_ops);
    read_conversation_focused = TRUE;
    presented_message_writes = 0;
    g_assert_true(
        dispatch_event.function(connection, &direct_caption, &accepted));
    g_assert_true(accepted);
    g_assert_cmpuint(presented_message_writes, ==, 1);
    g_assert_cmpuint(marker.calls, ==, 0);
    g_assert_cmpuint(g_hash_table_size(connection->pending_reads), ==, 0);

    purple_conversation_set_ui_ops(direct, &inline_media_conversation_ops);
    accepted = FALSE;
    g_assert_true(
        dispatch_event.function(connection, &direct_attachment, &accepted));
    g_assert_true(accepted);
    g_assert_cmpuint(inline_media_capture.writes, ==, 1);
    g_assert_true(inline_media_capture.conversation == direct);
    g_assert_cmpstr(inline_media_capture.sender, ==,
                    direct_attachment.peer_id);
    g_assert_cmpuint(inline_media_capture.flags, ==,
                     PURPLE_MESSAGE_RECV | PURPLE_MESSAGE_IMAGES);
    g_assert_cmpint(inline_media_capture.timestamp, ==, 2);
    g_assert_cmpuint(purple_imgstore_get_size(inline_media_capture.image), ==,
                     png_len);
    g_assert_cmpstr(purple_imgstore_get_filename(inline_media_capture.image),
                    ==, "direct.png");
    g_assert_cmpuint(marker.calls, ==, 1);
    g_assert_cmpstr(marker.recipient, ==, direct_attachment.peer_id);
    g_assert_cmpuint(marker.timestamp, ==, direct_attachment.timestamp_ms);
    g_assert_cmpuint(g_list_length(purple_xfers_get_all()), ==,
                     transfer_count);
    reset_inline_media_capture();

    purple_conversation_set_ui_ops(direct,
                                   &presented_message_conversation_ops);
    presented_message_writes = 0;
    purple_account_set_privacy_type(account, PURPLE_PRIVACY_DENY_ALL);
    direct_caption.request_id = 9002;
    direct_caption.timestamp_ms = 2001;
    accepted = FALSE;
    g_assert_true(
        dispatch_event.function(connection, &direct_caption, &accepted));
    g_assert_true(accepted);
    g_assert_cmpuint(presented_message_writes, ==, 0);
    g_assert_cmpuint(marker.calls, ==, 1);
    g_assert_cmpuint(g_hash_table_size(connection->pending_reads), ==, 0);

    direct_caption.flags = SIGNAL_EVENT_FLAG_OUTGOING;
    direct_caption.request_id = 0;
    direct_caption.timestamp_ms = 2002;
    accepted = FALSE;
    g_assert_true(
        dispatch_event.function(connection, &direct_caption, &accepted));
    g_assert_true(accepted);
    g_assert_cmpuint(presented_message_writes, ==, 1);
    g_assert_cmpuint(marker.calls, ==, 1);
    g_assert_cmpuint(g_hash_table_size(connection->pending_reads), ==, 0);
    direct_caption.flags = 0;

    purple_conversation_set_ui_ops(direct, &inline_media_conversation_ops);
    direct_attachment.request_id = 9003;
    direct_attachment.timestamp_ms = 2003;
    accepted = FALSE;
    g_assert_true(
        dispatch_event.function(connection, &direct_attachment, &accepted));
    g_assert_true(accepted);
    g_assert_cmpuint(inline_media_capture.writes, ==, 0);
    g_assert_cmpuint(g_list_length(purple_xfers_get_all()), ==,
                     transfer_count);
    g_assert_cmpuint(marker.calls, ==, 1);
    g_assert_cmpuint(g_hash_table_size(connection->pending_reads), ==, 0);
    purple_account_set_privacy_type(account, PURPLE_PRIVACY_ALLOW_ALL);

    accepted = FALSE;
    g_assert_true(dispatch_event.function(connection, &group_message,
                                          &accepted));
    g_assert_true(accepted);
    group = signal_group_sync_lookup_conversation(account,
                                                  group_attachment.chat_id);
    g_assert_nonnull(group);
    purple_conversation_set_ui_ops(group,
                                   &presented_message_conversation_ops);
    purple_conv_chat_ignore(PURPLE_CONV_CHAT(group),
                            group_attachment.peer_id);
    presented_message_writes = 0;
    accepted = FALSE;
    g_assert_true(dispatch_event.function(connection, &group_message_filtered,
                                          &accepted));
    g_assert_true(accepted);
    g_assert_cmpuint(presented_message_writes, ==, 0);
    g_assert_cmpuint(marker.calls, ==, 1);
    g_assert_cmpuint(g_hash_table_size(connection->pending_reads), ==, 0);

    purple_conversation_set_ui_ops(group, &inline_media_conversation_ops);
    accepted = FALSE;
    g_assert_true(dispatch_event.function(connection, &group_attachment,
                                          &accepted));
    g_assert_true(accepted);
    g_assert_cmpuint(inline_media_capture.writes, ==, 0);
    g_assert_cmpuint(g_list_length(purple_xfers_get_all()), ==,
                     transfer_count);
    g_assert_cmpuint(marker.calls, ==, 1);
    g_assert_cmpuint(g_hash_table_size(connection->pending_reads), ==, 0);

    purple_conv_chat_unignore(PURPLE_CONV_CHAT(group),
                              group_attachment.peer_id);
    accepted = FALSE;
    g_assert_true(dispatch_event.function(connection, &group_attachment,
                                          &accepted));
    g_assert_true(accepted);
    g_assert_cmpuint(inline_media_capture.writes, ==, 1);
    g_assert_true(inline_media_capture.conversation == group);
    g_assert_cmpstr(inline_media_capture.sender, ==,
                    group_attachment.peer_id);
    g_assert_cmpuint(inline_media_capture.flags, ==,
                     PURPLE_MESSAGE_RECV | PURPLE_MESSAGE_IMAGES);
    g_assert_cmpint(inline_media_capture.timestamp, ==, 3);
    g_assert_cmpstr(purple_imgstore_get_filename(inline_media_capture.image),
                    ==, "group.png");
    g_assert_cmpuint(g_list_length(purple_xfers_get_all()), ==,
                     transfer_count);
    g_assert_cmpuint(marker.calls, ==, 2);
    g_assert_cmpstr(marker.recipient, ==, group_attachment.peer_id);
    g_assert_cmpuint(marker.timestamp, ==, group_attachment.timestamp_ms);
    reset_inline_media_capture();

    connection->core = NULL;
    serv_got_chat_left(&gc,
                       purple_conv_chat_get_id(PURPLE_CONV_CHAT(group)));
    purple_conversation_destroy(group);
    purple_conversation_destroy(direct);
    protocol->close(&gc);
    purple_account_set_connection(account, NULL);
    purple_account_destroy(account);
    g_clear_pointer(&marker.recipient, g_free);
    read_conversation_focused = FALSE;
}

static void
test_pending_read_receipt_limit(PurplePlugin *plugin,
                                PurplePluginProtocolInfo *protocol)
{
    union {
        gpointer pointer;
        SignalDispatchEventFunc function;
    } dispatch_event = { 0 };
    PurpleAccount *account =
        purple_account_new("pending-read-limit", SIGNAL_PLUGIN_ID);
    PurpleConnection gc = {
        .prpl = plugin,
        .state = PURPLE_CONNECTED,
        .account = account,
    };
    SignalConnection *connection;
    PurpleConversation *conversation;
    SignalEvent message = {
        .abi_version = SIGNAL_CORE_ABI_VERSION,
        .struct_size = sizeof(SignalEvent),
        .kind = SIGNAL_EVENT_MESSAGE,
        .request_id = 1,
        .peer_id = "aci:pending-read-limit-contact",
        .text = "pending read limit probe",
    };
    gboolean accepted = FALSE;

    g_assert_true(g_module_symbol((GModule *)plugin->handle,
                                  "signal_dispatch_event",
                                  &dispatch_event.pointer));
    purple_account_set_connection(account, &gc);
    connection = new_transfer_connection(&gc);
    read_conversation_focused = FALSE;

    for (guint64 timestamp = 1; timestamp <= 5000; timestamp++) {
        message.timestamp_ms = timestamp;
        g_assert_true(dispatch_event.function(connection, &message, &accepted));
        g_assert_true(accepted);
    }
    g_assert_cmpuint(g_hash_table_size(connection->pending_reads), ==, 4096);
    g_assert_true(connection->pending_read_limit_warned);

    conversation = purple_find_conversation_with_account(
        PURPLE_CONV_TYPE_IM, message.peer_id, account);
    g_assert_nonnull(conversation);
    purple_conversation_destroy(conversation);
    protocol->close(&gc);
    purple_account_set_connection(account, NULL);
    purple_account_destroy(account);
}

static void
test_avatar_support(PurplePlugin *plugin, PurplePluginProtocolInfo *protocol)
{
    PurpleAccount *account =
        purple_account_new("avatar-probe", SIGNAL_PLUGIN_ID);
    PurpleConnection gc = {
        .prpl = plugin,
        .state = PURPLE_CONNECTED,
        .account = account,
    };
    SignalConnection *connection;
    union {
        gpointer pointer;
        SignalDispatchEventFunc function;
    } dispatch_event = { 0 };
    PurpleGroup *group;
    PurpleBuddy *buddy;
    PurpleChat *chat;
    const guint8 png_data[] = { 0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A };
    SignalEvent contact_avatar_event = {
        .abi_version = SIGNAL_CORE_ABI_VERSION,
        .struct_size = sizeof(SignalEvent),
        .kind = SIGNAL_EVENT_AVATAR,
        .peer_id = "aci:avatar-buddy-1",
        .data = png_data,
        .data_len = sizeof(png_data),
    };
    SignalEvent group_avatar_event = {
        .abi_version = SIGNAL_CORE_ABI_VERSION,
        .struct_size = sizeof(SignalEvent),
        .kind = SIGNAL_EVENT_AVATAR,
        .chat_id = "avatar-group-1",
        .data = png_data,
        .data_len = sizeof(png_data),
    };
    gboolean accepted = FALSE;

    g_assert_cmpstr(protocol->icon_spec.format, ==, "png,jpeg,gif");
    g_assert_cmpuint(protocol->icon_spec.max_filesize, ==, 10 * 1024 * 1024);
    g_assert_cmpint(protocol->icon_spec.scale_rules, ==, PURPLE_ICON_SCALE_DISPLAY);

    g_assert_true(g_module_symbol((GModule *)plugin->handle,
                                  "signal_dispatch_event",
                                  &dispatch_event.pointer));
    purple_account_set_connection(account, &gc);
    connection = new_transfer_connection(&gc);

    group = purple_group_new("Avatar Test Group");
    purple_blist_add_group(group, NULL);
    buddy = purple_buddy_new(account, "aci:avatar-buddy-1", "Avatar Buddy");
    purple_blist_add_buddy(buddy, NULL, group, NULL);
    chat = add_group_chat(account, group, "avatar-group-1", TRUE);

    g_assert_true(dispatch_event.function(connection, &contact_avatar_event, &accepted));
    g_assert_nonnull(purple_buddy_get_icon(buddy));

    g_assert_true(dispatch_event.function(connection, &group_avatar_event, &accepted));
    g_assert_true(purple_buddy_icons_node_has_custom_icon(PURPLE_BLIST_NODE(chat)));

    purple_blist_remove_buddy(buddy);
    purple_blist_remove_chat(chat);
    purple_blist_remove_group(group);

    protocol->close(&gc);
    purple_account_set_connection(account, NULL);
    purple_account_destroy(account);
}

static void
test_pending_transfer_disconnect(PurplePluginProtocolInfo *protocol,
                                 const char *user_dir)
{
    enum TransferCallback {
        CANCEL_TRANSFER,
        DENY_TRANSFER,
        ACCEPT_TRANSFER,
    };
    const struct {
        enum TransferCallback callback;
        gboolean disconnect_first;
    } cases[] = {
        { CANCEL_TRANSFER, FALSE },
        { DENY_TRANSFER, FALSE },
        { CANCEL_TRANSFER, TRUE },
        { DENY_TRANSFER, TRUE },
        { ACCEPT_TRANSFER, TRUE },
    };
    g_autofree char *filename =
        g_build_filename(user_dir, "pending-transfer.txt", NULL);

    g_assert_true(g_file_set_contents(filename, "content", -1, NULL));
    for (guint index = 0; index < G_N_ELEMENTS(cases); index++) {
        g_autofree char *username =
            g_strdup_printf("transfer-lifetime-%u", index);
        PurpleAccount *account = purple_account_new(username, SIGNAL_PLUGIN_ID);
        PurpleConnection gc = {
            .state = PURPLE_CONNECTED,
            .account = account,
        };
        SignalConnection *connection;
        SignalOutgoingAttachment *attachment;
        PurpleXfer *xfer;

        purple_account_set_connection(account, &gc);
        connection = new_transfer_connection(&gc);
        xfer = protocol->new_xfer(&gc, "aci:transfer-recipient");
        g_assert_nonnull(xfer);
        attachment = xfer->data;
        g_assert_nonnull(attachment);
        g_assert_true(g_hash_table_contains(
            connection->outgoing_attachment_contexts, attachment));

        if (cases[index].disconnect_first) {
            protocol->close(&gc);
            g_assert_null(xfer->data);
        }

        switch (cases[index].callback) {
        case CANCEL_TRANSFER:
            purple_xfer_cancel_local(xfer);
            break;
        case DENY_TRANSFER:
            purple_xfer_request_denied(xfer);
            break;
        case ACCEPT_TRANSFER:
            purple_xfer_ref(xfer);
            purple_xfer_request_accepted(xfer, filename);
            g_assert_true(purple_xfer_is_canceled(xfer));
            purple_xfer_unref(xfer);
            break;
        }
        if (!cases[index].disconnect_first) {
            g_assert_cmpuint(
                g_hash_table_size(connection->outgoing_attachment_contexts),
                ==, 0);
            protocol->close(&gc);
        }
        purple_account_set_connection(account, NULL);
        purple_account_destroy(account);
    }
}

static guint start_reference_floor;
static gboolean start_had_temporary_reference;
static PurpleConnection *closing_gc;
static SignalConnection *closing_connection;
static void (*closing_cancel_send)(PurpleXfer *xfer);
static gboolean close_detached_protocol_data;
static gboolean close_marked_connection_closing;

static void
cancel_transfer_during_start(PurpleXfer *xfer, int fd, const char *ip,
                             unsigned int port)
{
    (void)fd;
    (void)ip;
    (void)port;
    start_had_temporary_reference = xfer->ref >= start_reference_floor;
    purple_xfer_cancel_local(xfer);
}

static void
observe_close_order(PurpleXfer *xfer)
{
    g_assert_nonnull(closing_gc);
    g_assert_nonnull(closing_connection);
    g_assert_nonnull(closing_cancel_send);
    close_detached_protocol_data =
        purple_connection_get_protocol_data(closing_gc) == NULL;
    close_marked_connection_closing = closing_connection->closing;
    closing_cancel_send(xfer);
}

static void
test_synchronous_start_failure(PurplePluginProtocolInfo *protocol,
                               const char *user_dir)
{
    g_autofree char *filename =
        g_build_filename(user_dir, "start-failure-transfer.txt", NULL);
    PurpleAccount *account =
        purple_account_new("start-failure-transfer", SIGNAL_PLUGIN_ID);
    PurpleConnection gc = {
        .state = PURPLE_CONNECTED,
        .account = account,
    };
    SignalConnection *connection;
    PurpleXfer *xfer;

    g_assert_true(g_file_set_contents(filename, "content", -1, NULL));
    purple_account_set_connection(account, &gc);
    connection = new_transfer_connection(&gc);
    connection->start_xfer = cancel_transfer_during_start;
    xfer = protocol->new_xfer(&gc, "aci:transfer-recipient");
    g_assert_nonnull(xfer);
    purple_xfer_ref(xfer);
    start_reference_floor = xfer->ref + 1;
    start_had_temporary_reference = FALSE;

    purple_xfer_request_accepted(xfer, filename);

    g_assert_true(start_had_temporary_reference);
    g_assert_true(purple_xfer_is_canceled(xfer));
    g_assert_null(xfer->data);
    g_assert_cmpuint(
        g_hash_table_size(connection->outgoing_attachment_contexts),
        ==, 0);
    purple_xfer_unref(xfer);
    protocol->close(&gc);
    purple_account_set_connection(account, NULL);
    purple_account_destroy(account);
}

static void
test_started_transfer_disconnect(PurplePluginProtocolInfo *protocol)
{
    PurpleAccount *account =
        purple_account_new("started-transfer", SIGNAL_PLUGIN_ID);
    PurpleConnection gc = {
        .state = PURPLE_CONNECTED,
        .account = account,
    };
    SignalConnection *connection;
    SignalOutgoingAttachment *attachment;
    PurpleXfer *xfer;
    guint64 *key;

    purple_account_set_connection(account, &gc);
    connection = new_transfer_connection(&gc);
    xfer = protocol->new_xfer(&gc, "aci:transfer-recipient");
    g_assert_nonnull(xfer);
    purple_xfer_ref(xfer);

    attachment = xfer->data;
    g_assert_nonnull(attachment);
    attachment->request_id = 42;
    xfer->status = PURPLE_XFER_STATUS_STARTED;
    closing_gc = &gc;
    closing_connection = connection;
    closing_cancel_send = xfer->ops.cancel_send;
    close_detached_protocol_data = FALSE;
    close_marked_connection_closing = FALSE;
    purple_xfer_set_cancel_send_fnc(xfer, observe_close_order);
    key = g_new(guint64, 1);
    *key = attachment->request_id;
    purple_xfer_ref(xfer);
    g_hash_table_insert(connection->outgoing_attachments, key, xfer);

    protocol->close(&gc);

    g_assert_true(close_detached_protocol_data);
    g_assert_true(close_marked_connection_closing);
    g_assert_true(purple_xfer_is_canceled(xfer));
    g_assert_null(xfer->data);
    closing_gc = NULL;
    closing_connection = NULL;
    closing_cancel_send = NULL;
    purple_xfer_unref(xfer);
    purple_account_set_connection(account, NULL);
    purple_account_destroy(account);
}

static void
test_vanished_transfer_cleanup(PurplePluginProtocolInfo *protocol,
                               const char *user_dir)
{
    g_autofree char *filename =
        g_build_filename(user_dir, "vanished-transfer.txt", NULL);
    PurpleAccount *account =
        purple_account_new("vanished-transfer", SIGNAL_PLUGIN_ID);
    PurpleConnection gc = {
        .state = PURPLE_CONNECTED,
        .account = account,
    };
    SignalConnection *connection;
    SignalOutgoingAttachment *attachment;
    PurpleXfer *xfer;

    (void)g_remove(filename);
    purple_account_set_connection(account, &gc);
    connection = new_transfer_connection(&gc);
    xfer = protocol->new_xfer(&gc, "aci:transfer-recipient");
    g_assert_nonnull(xfer);
    attachment = xfer->data;

    purple_xfer_request_accepted(xfer, filename);
    g_assert_true(g_hash_table_contains(
        connection->outgoing_attachment_contexts, attachment));

    protocol->close(&gc);
    purple_account_set_connection(account, NULL);
    purple_account_destroy(account);
}

int
main(int argc, char **argv)
{
    PurplePlugin *plugin;
    PurplePluginProtocolInfo *protocol;
    GList *chat_info;
    struct proto_chat_entry *group_key_entry;
    GHashTable *components;
    PurpleAccount *sync_account;
    PurpleGroup *sync_group;
    PurpleGroup *default_buddy_group;
    PurpleGroup *default_chat_group;
    PurpleChat *selected;
    PurpleChat *user_chat;
    guint removed = 0;
    g_autofree char *chat_name = NULL;
    g_autoptr(GError) error = NULL;
    g_autofree char *user_dir = NULL;
    union {
        gpointer pointer;
        SignalConnectionNewFunc function;
    } resolved_connection_new = { 0 };

    g_assert_cmpint(argc, ==, 2);
    user_dir = g_dir_make_tmp("signal-purple-test-XXXXXX", &error);
    g_assert_no_error(error);
    purple_util_set_user_dir(user_dir);
    purple_eventloop_set_ui_ops(&event_loop_ops);
    purple_core_set_ui_ops(&core_ops);
    g_assert_true(purple_core_init("signal-purple-tests"));
    purple_set_blist(purple_blist_new());

    plugin = purple_plugins_find_with_id(SIGNAL_PLUGIN_ID);
    if (plugin != NULL) {
        if (purple_plugin_is_loaded(plugin))
            g_assert_true(purple_plugin_unload(plugin));
        purple_plugin_destroy(plugin);
    }
    plugin = purple_plugin_probe(argv[1]);
    g_assert_nonnull(plugin);
    purple_plugins_probe("signal-purple-test-no-such-extension");
    plugin = purple_plugins_find_with_id(SIGNAL_PLUGIN_ID);
    g_assert_nonnull(plugin);
    g_assert_cmpstr(purple_plugin_get_id(plugin), ==, SIGNAL_PLUGIN_ID);
    g_assert_cmpstr(purple_plugin_get_name(plugin), ==, "Signal");
    g_assert_cmpstr(purple_plugin_get_version(plugin), ==,
                    EXPECTED_PLUGIN_VERSION);
    g_assert_cmpint(plugin->info->type, ==, PURPLE_PLUGIN_PROTOCOL);
    if (!purple_plugin_is_loaded(plugin))
        g_assert_true(purple_plugin_load(plugin));
    g_assert_true(g_module_symbol((GModule *)plugin->handle,
                                  "signal_connection_new",
                                  &resolved_connection_new.pointer));
    connection_new = resolved_connection_new.function;

    protocol = PURPLE_PLUGIN_PROTOCOL_INFO(plugin);
    g_assert_nonnull(protocol);
    g_assert_cmpuint(protocol->struct_size, ==,
                     sizeof(PurplePluginProtocolInfo));
    g_assert_nonnull(protocol->list_icon);
    g_assert_nonnull(protocol->status_types);
    g_assert_nonnull(protocol->login);
    g_assert_nonnull(protocol->close);
    g_assert_nonnull(protocol->send_im);
    g_assert_nonnull(protocol->send_typing);
    g_assert_nonnull(protocol->can_receive_file);
    g_assert_nonnull(protocol->send_file);
    g_assert_nonnull(protocol->new_xfer);
    g_assert_nonnull(protocol->blist_node_menu);
    g_assert_nonnull(protocol->chat_info);
    g_assert_nonnull(protocol->chat_info_defaults);
    g_assert_nonnull(protocol->join_chat);
    g_assert_nonnull(protocol->get_chat_name);
    g_assert_nonnull(protocol->chat_send);
    g_assert_nonnull(protocol->chat_can_receive_file);
    g_assert_nonnull(protocol->chat_send_file);
    g_assert_nonnull(protocol->find_blist_chat);
    g_assert_nonnull(protocol->get_cb_alias);
    test_connection_owned_resource_cleanup(protocol);
    test_avatar_support(plugin, protocol);
    test_standard_conversation_logging(plugin, protocol);
    test_pending_read_receipt_admission(plugin, protocol);
    test_inline_attachment_routing(plugin, protocol);
    test_pending_read_receipt_limit(plugin, protocol);
    test_pending_transfer_disconnect(protocol, user_dir);
    test_synchronous_start_failure(protocol, user_dir);
    test_started_transfer_disconnect(protocol);
    test_vanished_transfer_cleanup(protocol, user_dir);

    chat_info = protocol->chat_info(NULL);
    g_assert_cmpuint(g_list_length(chat_info), ==, 1);
    group_key_entry = chat_info->data;
    g_assert_cmpstr(group_key_entry->identifier, ==, "group-id");
    g_assert_true(group_key_entry->required);
    g_free(group_key_entry);
    g_list_free(chat_info);
    chat_info = protocol->chat_info(NULL);
    g_assert_cmpuint(g_list_length(chat_info), ==, 1);
    g_free(chat_info->data);
    g_list_free(chat_info);

    components = protocol->chat_info_defaults(NULL, "001122");
    g_assert_cmpstr(g_hash_table_lookup(components, "group-id"), ==,
                    "001122");
    chat_name = protocol->get_chat_name(components);
    g_assert_cmpstr(chat_name, ==, "001122");
    g_hash_table_unref(components);

    sync_account = purple_account_new("group-sync-test", SIGNAL_PLUGIN_ID);
    if (purple_account_get_status_types(sync_account) == NULL)
        purple_account_set_status_types(sync_account,
                                        protocol->status_types(sync_account));
    test_default_blist_placement(sync_account, &default_buddy_group,
                                 &default_chat_group);
    test_buddy_legacy_migration(sync_account, default_buddy_group);
    test_buddy_legacy_adoption(protocol, sync_account,
                               default_buddy_group);
    test_chat_legacy_migration(sync_account, default_chat_group);

    sync_group = purple_group_new("Group sync tests");
    purple_blist_add_group(sync_group, NULL);
    test_group_title_tracking(sync_account, sync_group);
    test_remove_managed_group_chats(sync_account, sync_group);
    test_group_conversation_identity(plugin, protocol, sync_account,
                                     sync_group);
    add_group_chat(sync_account, sync_group, "stable-id", TRUE);
    add_group_chat(sync_account, sync_group, "stable-id", TRUE);
    add_group_chat(sync_account, sync_group, "stable-id", TRUE);
    selected = signal_group_sync_find_chat(sync_account, "stable-id",
                                           &removed);
    g_assert_nonnull(selected);
    g_assert_cmpuint(removed, ==, 2);

    user_chat = add_group_chat(sync_account, sync_group, "stable-id", FALSE);
    add_group_chat(sync_account, sync_group, "stable-id", TRUE);
    selected = signal_group_sync_find_chat(sync_account, "stable-id",
                                           &removed);
    g_assert_true(selected == user_chat);
    g_assert_cmpuint(removed, ==, 4);

    purple_core_quit();
    remove_tree(user_dir);
    return 0;
}
