/* SPDX-License-Identifier: GPL-3.0-or-later */
#include "attachment_file.h"

#include <errno.h>
#include <fcntl.h>
#include <glib/gstdio.h>
#include <sys/stat.h>
#include <unistd.h>

#define SIGNAL_ATTACHMENT_READ_CHUNK (64u * 1024u)

static void
signal_set_file_error(GError **error, int error_number, const char *operation)
{
    g_set_error(error, G_IO_ERROR, g_io_error_from_errno(error_number),
                "Could not %s attachment file: %s", operation,
                g_strerror(error_number));
}

static void
signal_close_ignoring_error(int descriptor)
{
    (void)g_close(descriptor, NULL);
}

GBytes *
signal_read_bounded_file(const char *path, gsize maximum_bytes, GError **error)
{
    guint8 buffer[SIGNAL_ATTACHMENT_READ_CHUNK];
    g_autoptr(GByteArray) contents = NULL;
    struct stat metadata;
    int descriptor;

    g_return_val_if_fail(error == NULL || *error == NULL, NULL);
    if (path == NULL || path[0] == '\0' || maximum_bytes == 0 ||
        maximum_bytes > G_MAXUINT) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "Attachment file parameters are invalid");
        return NULL;
    }

    descriptor = g_open(path, O_RDONLY | O_CLOEXEC | O_NONBLOCK, 0);
    if (descriptor < 0) {
        int open_error = errno;

        signal_set_file_error(error, open_error, "open");
        return NULL;
    }

    if (fstat(descriptor, &metadata) != 0) {
        int stat_error = errno;

        signal_close_ignoring_error(descriptor);
        signal_set_file_error(error, stat_error, "inspect");
        return NULL;
    }
    if (!S_ISREG(metadata.st_mode)) {
        signal_close_ignoring_error(descriptor);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_REGULAR_FILE,
                            "Attachment path is not a regular file");
        return NULL;
    }
    if (metadata.st_size < 0 ||
        (guint64)metadata.st_size > (guint64)maximum_bytes) {
        signal_close_ignoring_error(descriptor);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_MESSAGE_TOO_LARGE,
                            "Attachment file exceeds the size limit");
        return NULL;
    }

    contents = g_byte_array_sized_new((guint)metadata.st_size);
    while (TRUE) {
        gsize remaining = maximum_bytes - contents->len;
        gsize requested = sizeof(buffer);
        ssize_t bytes_read;

        if (remaining < requested)
            requested = remaining + 1;
        bytes_read = read(descriptor, buffer, requested);
        if (bytes_read < 0) {
            int read_error = errno;

            if (read_error == EINTR)
                continue;
            signal_close_ignoring_error(descriptor);
            signal_set_file_error(error, read_error, "read");
            return NULL;
        }
        if (bytes_read == 0)
            break;
        if ((gsize)bytes_read > remaining) {
            signal_close_ignoring_error(descriptor);
            g_set_error_literal(error, G_IO_ERROR,
                                G_IO_ERROR_MESSAGE_TOO_LARGE,
                                "Attachment file exceeds the size limit");
            return NULL;
        }
        g_byte_array_append(contents, buffer, (guint)bytes_read);
    }

    signal_close_ignoring_error(descriptor);
    if (contents->len == 0) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "Attachment file is empty");
        return NULL;
    }

    return g_byte_array_free_to_bytes(g_steal_pointer(&contents));
}
