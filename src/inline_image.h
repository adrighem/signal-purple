/* SPDX-License-Identifier: GPL-3.0-or-later */
#ifndef SIGNAL_INLINE_IMAGE_H
#define SIGNAL_INLINE_IMAGE_H

#include <glib.h>
#include <purple.h>

typedef void (*SignalInlineImageWriter)(PurpleConnection *gc, int chat_id,
                                        const char *sender,
                                        PurpleMessageFlags flags,
                                        const char *message,
                                        time_t timestamp);

gboolean signal_inline_image_is_supported(const char *mime_type,
                                          const guint8 *data, gsize size);

gboolean signal_inline_image_dimensions_are_supported(int width, int height);

gboolean signal_inline_image_deliver_with_writer(
    PurpleConnection *gc, int chat_id, const char *sender,
    const char *filename, const char *mime_type, const guint8 *data,
    gsize size, time_t timestamp, SignalInlineImageWriter writer);

gboolean signal_inline_image_deliver(PurpleConnection *gc, int chat_id,
                                     const char *sender,
                                     const char *filename,
                                     const char *mime_type,
                                     const guint8 *data, gsize size,
                                     time_t timestamp);

#endif
