/* SPDX-License-Identifier: GPL-3.0-or-later */
#include "inline_image.h"

#include <gdk-pixbuf/gdk-pixbuf.h>
#include <string.h>

#define SIGNAL_INLINE_IMAGE_MAX_EDGE 8192
#define SIGNAL_INLINE_IMAGE_MAX_PIXELS \
    (G_GUINT64_CONSTANT(16) * 1000u * 1000u)
#define SIGNAL_INLINE_IMAGE_VALIDATION_CHUNK 4096u
#define SIGNAL_INLINE_GIF_MAX_BYTES (8u * 1024u * 1024u)
#define SIGNAL_INLINE_GIF_MAX_PIXEL_FRAMES \
    (G_GUINT64_CONSTANT(8) * 1000u * 1000u)

typedef struct {
    gboolean dimensions_seen;
    gboolean dimensions_supported;
} SignalInlineImageValidation;

static gboolean
signal_inline_image_has_prefix(const guint8 *data, gsize size,
                               const guint8 *prefix, gsize prefix_size)
{
    return data != NULL && size >= prefix_size &&
           memcmp(data, prefix, prefix_size) == 0;
}

static guint16
signal_inline_gif_read_u16(const guint8 *data)
{
    return (guint16)((guint)data[0] | ((guint)data[1] << 8));
}

static gboolean
signal_inline_gif_advance(gsize *offset, gsize amount, gsize size)
{
    if (*offset > size || amount > size - *offset)
        return FALSE;
    *offset += amount;
    return TRUE;
}

static gboolean
signal_inline_gif_skip_sub_blocks(const guint8 *data, gsize size,
                                  gsize *offset)
{
    while (*offset < size) {
        guint8 block_size = data[*offset];

        *offset += 1;
        if (block_size == 0)
            return TRUE;
        if (!signal_inline_gif_advance(offset, block_size, size))
            return FALSE;
    }
    return FALSE;
}

static gboolean
signal_inline_gif_is_bounded(const guint8 *data, gsize size)
{
    static const guint8 gif87a_prefix[] = {'G', 'I', 'F', '8', '7', 'a'};
    static const guint8 gif89a_prefix[] = {'G', 'I', 'F', '8', '9', 'a'};
    guint16 canvas_width;
    guint16 canvas_height;
    guint64 canvas_pixels;
    guint64 frames = 0;
    gsize offset = 13;
    guint8 packed;

    if (size < 13 || size > SIGNAL_INLINE_GIF_MAX_BYTES ||
        (!signal_inline_image_has_prefix(data, size, gif87a_prefix,
                                         sizeof(gif87a_prefix)) &&
         !signal_inline_image_has_prefix(data, size, gif89a_prefix,
                                         sizeof(gif89a_prefix))))
        return FALSE;

    canvas_width = signal_inline_gif_read_u16(data + 6);
    canvas_height = signal_inline_gif_read_u16(data + 8);
    if (!signal_inline_image_dimensions_are_supported(canvas_width,
                                                      canvas_height))
        return FALSE;
    canvas_pixels = (guint64)canvas_width * (guint64)canvas_height;

    packed = data[10];
    if ((packed & 0x80u) != 0) {
        gsize color_table_size =
            (gsize)3u << ((gsize)(packed & 0x07u) + 1u);

        if (!signal_inline_gif_advance(&offset, color_table_size, size))
            return FALSE;
    }

    while (offset < size) {
        guint8 marker = data[offset++];

        if (marker == 0x3bu)
            return frames > 0;
        if (marker == 0x21u) {
            if (!signal_inline_gif_advance(&offset, 1, size) ||
                !signal_inline_gif_skip_sub_blocks(data, size, &offset))
                return FALSE;
            continue;
        }
        if (marker == 0x2cu) {
            guint16 left;
            guint16 top;
            guint16 width;
            guint16 height;
            guint8 image_packed;

            if (offset > size || 9u > size - offset)
                return FALSE;
            left = signal_inline_gif_read_u16(data + offset);
            top = signal_inline_gif_read_u16(data + offset + 2);
            width = signal_inline_gif_read_u16(data + offset + 4);
            height = signal_inline_gif_read_u16(data + offset + 6);
            image_packed = data[offset + 8];
            offset += 9;

            if (width == 0 || height == 0 ||
                (guint32)left + (guint32)width > canvas_width ||
                (guint32)top + (guint32)height > canvas_height)
                return FALSE;
            frames += 1;
            if (frames > SIGNAL_INLINE_GIF_MAX_PIXEL_FRAMES / canvas_pixels)
                return FALSE;

            if ((image_packed & 0x80u) != 0) {
                gsize color_table_size =
                    (gsize)3u << ((gsize)(image_packed & 0x07u) + 1u);

                if (!signal_inline_gif_advance(&offset, color_table_size,
                                               size))
                    return FALSE;
            }
            if (offset >= size || data[offset] < 2u || data[offset] > 8u)
                return FALSE;
            offset += 1;
            if (!signal_inline_gif_skip_sub_blocks(data, size, &offset))
                return FALSE;
            continue;
        }
        return FALSE;
    }
    return FALSE;
}

gboolean
signal_inline_image_dimensions_are_supported(int width, int height)
{
    return width > 0 && height > 0 && width <= SIGNAL_INLINE_IMAGE_MAX_EDGE &&
           height <= SIGNAL_INLINE_IMAGE_MAX_EDGE &&
           (guint64)width * (guint64)height <=
               SIGNAL_INLINE_IMAGE_MAX_PIXELS;
}

static void
signal_inline_image_size_prepared(GdkPixbufLoader *loader, int width,
                                  int height, gpointer user_data)
{
    SignalInlineImageValidation *validation = user_data;

    validation->dimensions_seen = TRUE;
    validation->dimensions_supported =
        signal_inline_image_dimensions_are_supported(width, height);
    if (!validation->dimensions_supported)
        gdk_pixbuf_loader_set_size(loader, 1, 1);
}

static const char *
signal_inline_image_type(const char *mime_type, const guint8 *data,
                         gsize size)
{
    static const guint8 jpeg_prefix[] = {0xff, 0xd8, 0xff};
    static const guint8 png_prefix[] = {0x89, 0x50, 0x4e, 0x47,
                                        0x0d, 0x0a, 0x1a, 0x0a};

    if (mime_type == NULL || data == NULL || size == 0)
        return NULL;
    if (g_ascii_strcasecmp(mime_type, "image/jpeg") == 0 &&
        signal_inline_image_has_prefix(data, size, jpeg_prefix,
                                       sizeof(jpeg_prefix)))
        return "jpeg";
    if (g_ascii_strcasecmp(mime_type, "image/png") == 0 &&
        signal_inline_image_has_prefix(data, size, png_prefix,
                                       sizeof(png_prefix)))
        return "png";
    if (g_ascii_strcasecmp(mime_type, "image/gif") == 0 &&
        signal_inline_gif_is_bounded(data, size))
        return "gif";
    return NULL;
}

gboolean
signal_inline_image_is_supported(const char *mime_type, const guint8 *data,
                                 gsize size)
{
    SignalInlineImageValidation validation = {0};
    g_autoptr(GdkPixbufLoader) loader = NULL;
    g_autoptr(GError) error = NULL;
    const char *image_type;
    gsize offset = 0;
    gboolean complete = TRUE;
    gboolean closed;

    image_type = signal_inline_image_type(mime_type, data, size);
    if (image_type == NULL)
        return FALSE;
    loader = gdk_pixbuf_loader_new_with_type(image_type, &error);
    if (loader == NULL)
        return FALSE;
    g_signal_connect(loader, "size-prepared",
                     G_CALLBACK(signal_inline_image_size_prepared),
                     &validation);

    while (offset < size) {
        gsize chunk = MIN(size - offset,
                          (gsize)SIGNAL_INLINE_IMAGE_VALIDATION_CHUNK);

        if (!gdk_pixbuf_loader_write(loader, data + offset, chunk, &error) ||
            (validation.dimensions_seen &&
             !validation.dimensions_supported)) {
            complete = FALSE;
            break;
        }
        offset += chunk;
    }
    g_clear_error(&error);
    closed = gdk_pixbuf_loader_close(loader, &error);
    return complete && closed && validation.dimensions_seen &&
           validation.dimensions_supported &&
           gdk_pixbuf_loader_get_pixbuf(loader) != NULL;
}

gboolean
signal_inline_image_deliver_with_writer(
    PurpleConnection *gc, int chat_id, const char *sender,
    const char *filename, const char *mime_type, const guint8 *data,
    gsize size, time_t timestamp, SignalInlineImageWriter writer)
{
    g_autofree char *markup = NULL;
    int image_id;

    if (gc == NULL || sender == NULL || sender[0] == '\0' || writer == NULL ||
        !signal_inline_image_is_supported(mime_type, data, size))
        return FALSE;

    image_id = purple_imgstore_add_with_id(g_memdup2(data, size), size,
                                           filename);
    if (image_id == 0)
        return FALSE;

    markup = g_strdup_printf("<img id=\"%d\">", image_id);
    /* Purple UIs resolve the image ID synchronously and retain their own
     * reference while the message remains displayed. */
    writer(gc, chat_id, sender,
           PURPLE_MESSAGE_RECV | PURPLE_MESSAGE_IMAGES, markup, timestamp);
    purple_imgstore_unref_by_id(image_id);
    return purple_imgstore_find_by_id(image_id) != NULL;
}

gboolean
signal_inline_image_deliver(PurpleConnection *gc, int chat_id,
                            const char *sender, const char *filename,
                            const char *mime_type, const guint8 *data,
                            gsize size, time_t timestamp)
{
    return signal_inline_image_deliver_with_writer(
        gc, chat_id, sender, filename, mime_type, data, size, timestamp,
        serv_got_chat_in);
}
