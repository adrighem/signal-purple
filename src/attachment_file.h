/* SPDX-License-Identifier: GPL-3.0-or-later */
#ifndef SIGNAL_ATTACHMENT_FILE_H
#define SIGNAL_ATTACHMENT_FILE_H

#include <gio/gio.h>

GBytes *signal_read_bounded_file(const char *path, gsize maximum_bytes,
                                 GError **error);

#endif
