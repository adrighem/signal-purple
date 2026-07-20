/* SPDX-License-Identifier: GPL-3.0-or-later */
#include <gdk-pixbuf/gdk-pixbuf.h>
#include <glib.h>
#include <glib/gstdio.h>
#include <purple.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>

#include "inline_image.h"

typedef struct {
    guint calls;
    PurpleConnection *gc;
    int chat_id;
    int image_id;
    char *sender;
    char *markup;
    PurpleMessageFlags flags;
    time_t timestamp;
    PurpleStoredImage *image;
} ReceivedImage;

typedef struct {
    PurpleInputFunction function;
    gpointer user_data;
} InputClosure;

static ReceivedImage received;
static int unretained_image_id;

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
        g_hash_table_insert(info, "name", "signal-purple inline image tests");
        g_hash_table_insert(info, "version", "0");
    }
    return info;
}

static PurpleCoreUiOps core_ops = {
    .get_ui_info = get_ui_info,
};

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

static guint8 *
encoded_image(const char *format, int width, int height, gsize *size)
{
    g_autoptr(GdkPixbuf) pixbuf = NULL;
    g_autoptr(GError) error = NULL;
    char *data = NULL;

    pixbuf = gdk_pixbuf_new(GDK_COLORSPACE_RGB, FALSE, 8, width, height);
    g_assert_nonnull(pixbuf);
    gdk_pixbuf_fill(pixbuf, 0x336699ffu);
    g_assert_true(gdk_pixbuf_save_to_buffer(pixbuf, &data, size, format,
                                            &error, NULL));
    g_assert_no_error(error);
    return (guint8 *)data;
}

static void
capture_image(PurpleConnection *gc, int chat_id, const char *sender,
              PurpleMessageFlags flags, const char *message,
              time_t timestamp)
{
    PurpleStoredImage *image;
    int image_id = 0;

    g_assert_cmpint(sscanf(message, "<img id=\"%d\">", &image_id), ==, 1);
    image = purple_imgstore_find_by_id(image_id);
    g_assert_nonnull(image);

    received.calls++;
    received.gc = gc;
    received.chat_id = chat_id;
    received.image_id = image_id;
    received.sender = g_strdup(sender);
    received.markup = g_strdup(message);
    received.flags = flags;
    received.timestamp = timestamp;
    received.image = purple_imgstore_ref(image);
}

static void
observe_image_without_ref(PurpleConnection *gc, int chat_id,
                          const char *sender, PurpleMessageFlags flags,
                          const char *message, time_t timestamp)
{
    int image_id = 0;

    (void)gc;
    (void)chat_id;
    (void)sender;
    (void)flags;
    (void)timestamp;
    g_assert_cmpint(sscanf(message, "<img id=\"%d\">", &image_id), ==, 1);
    g_assert_nonnull(purple_imgstore_find_by_id(image_id));
    unretained_image_id = image_id;
}

static void
reset_received(void)
{
    g_clear_pointer(&received.sender, g_free);
    g_clear_pointer(&received.markup, g_free);
    g_clear_pointer(&received.image, purple_imgstore_unref);
    received = (ReceivedImage){0};
}

static void
test_supported_formats(void)
{
    static const guint8 corrupt_jpeg[] = {0xff, 0xd8, 0xff, 0xe0};
    static const guint8 corrupt_png[] = {0x89, 0x50, 0x4e, 0x47, 0x0d,
                                         0x0a, 0x1a, 0x0a, 0x00};
    static const guint8 gif[] = {'G', 'I', 'F', '8', '9', 'a'};
    g_autofree guint8 *jpeg = NULL;
    g_autofree guint8 *png = NULL;
    g_autofree guint8 *oversized_png = NULL;
    gsize jpeg_size = 0;
    gsize png_size = 0;
    gsize oversized_png_size = 0;

    jpeg = encoded_image("jpeg", 2, 2, &jpeg_size);
    png = encoded_image("png", 2, 2, &png_size);
    oversized_png = encoded_image("png", 8193, 1, &oversized_png_size);

    g_assert_true(
        signal_inline_image_is_supported("image/jpeg", jpeg, jpeg_size));
    g_assert_true(
        signal_inline_image_is_supported("IMAGE/JPEG", jpeg, jpeg_size));
    g_assert_true(
        signal_inline_image_is_supported("image/png", png, png_size));
    g_assert_false(
        signal_inline_image_is_supported("image/png", jpeg, jpeg_size));
    g_assert_false(
        signal_inline_image_is_supported("image/jpeg", png, png_size));
    g_assert_false(signal_inline_image_is_supported(
        "image/jpeg", corrupt_jpeg, sizeof(corrupt_jpeg)));
    g_assert_false(signal_inline_image_is_supported(
        "image/png", corrupt_png, sizeof(corrupt_png)));
    g_assert_false(signal_inline_image_is_supported("image/gif", gif,
                                                     sizeof(gif)));
    g_assert_false(
        signal_inline_image_is_supported("text/plain", png, png_size));
    g_assert_false(signal_inline_image_is_supported(NULL, png, png_size));
    g_assert_false(signal_inline_image_is_supported("image/png", NULL, 0));
    g_assert_false(signal_inline_image_is_supported(
        "image/png", oversized_png, oversized_png_size));

    g_assert_true(signal_inline_image_dimensions_are_supported(1, 1));
    g_assert_true(signal_inline_image_dimensions_are_supported(4000, 4000));
    g_assert_true(signal_inline_image_dimensions_are_supported(8000, 2000));
    g_assert_false(signal_inline_image_dimensions_are_supported(0, 1));
    g_assert_false(signal_inline_image_dimensions_are_supported(1, -1));
    g_assert_false(signal_inline_image_dimensions_are_supported(8193, 1));
    g_assert_false(signal_inline_image_dimensions_are_supported(4001, 4000));
}

static void
test_group_delivery(void)
{
    PurpleConnection connection = {0};
    PurpleStoredImage *stored;
    g_autofree char *expected_markup = NULL;
    g_autofree guint8 *png = NULL;
    gsize png_size = 0;
    const int chat_id = 73;
    const time_t timestamp = (time_t)1721400000;

    png = encoded_image("png", 2, 2, &png_size);
    g_assert_true(signal_inline_image_deliver_with_writer(
        &connection, chat_id, "Peter", "shared-image.png", "image/png", png,
        png_size, timestamp, capture_image));

    g_assert_cmpuint(received.calls, ==, 1);
    g_assert_true(received.gc == &connection);
    g_assert_cmpint(received.chat_id, ==, chat_id);
    g_assert_cmpstr(received.sender, ==, "Peter");
    g_assert_cmpint(received.timestamp, ==, timestamp);
    g_assert_cmpuint(received.flags, ==,
                     PURPLE_MESSAGE_RECV | PURPLE_MESSAGE_IMAGES);
    expected_markup = g_strdup_printf("<img id=\"%d\">", received.image_id);
    g_assert_cmpstr(received.markup, ==, expected_markup);

    stored = purple_imgstore_find_by_id(received.image_id);
    g_assert_true(stored == received.image);
    g_assert_cmpuint(purple_imgstore_get_size(stored), ==, png_size);
    g_assert_cmpmem(purple_imgstore_get_data(stored),
                    purple_imgstore_get_size(stored), png, png_size);
    g_assert_cmpstr(purple_imgstore_get_filename(stored), ==,
                    "shared-image.png");

    purple_imgstore_unref(g_steal_pointer(&received.image));
    g_assert_null(purple_imgstore_find_by_id(received.image_id));

    g_assert_false(signal_inline_image_deliver_with_writer(
        &connection, chat_id, "Peter", "spoofed.png", "image/png",
        (const guint8 *)"not a png", strlen("not a png"), timestamp,
        capture_image));
    g_assert_cmpuint(received.calls, ==, 1);

    unretained_image_id = 0;
    g_assert_false(signal_inline_image_deliver_with_writer(
        &connection, chat_id, "Peter", "unretained.png", "image/png", png,
        png_size, timestamp, observe_image_without_ref));
    g_assert_cmpint(unretained_image_id, >, 0);
    g_assert_null(purple_imgstore_find_by_id(unretained_image_id));
    reset_received();
}

int
main(void)
{
    g_autofree char *user_dir = NULL;
    g_autoptr(GError) error = NULL;

    user_dir = g_dir_make_tmp("signal-purple-inline-image-test-XXXXXX",
                              &error);
    g_assert_no_error(error);
    purple_util_set_user_dir(user_dir);
    purple_eventloop_set_ui_ops(&event_loop_ops);
    purple_core_set_ui_ops(&core_ops);
    g_assert_true(purple_core_init("signal-purple-inline-image-tests"));
    test_supported_formats();
    test_group_delivery();
    purple_core_quit();
    remove_tree(user_dir);
    return 0;
}
