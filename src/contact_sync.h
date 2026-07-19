/* SPDX-License-Identifier: GPL-3.0-or-later */
#ifndef SIGNAL_CONTACT_SYNC_H
#define SIGNAL_CONTACT_SYNC_H

#include <glib.h>

typedef struct {
    GHashTable *seen;
    gboolean active;
} SignalContactSync;

void signal_contact_sync_init(SignalContactSync *sync);
void signal_contact_sync_clear(SignalContactSync *sync);
void signal_contact_sync_begin(SignalContactSync *sync);
void signal_contact_sync_mark(SignalContactSync *sync, const char *identifier);
gboolean signal_contact_sync_should_remove(const SignalContactSync *sync,
                                           const char *identifier);
void signal_contact_sync_end(SignalContactSync *sync);

#endif
