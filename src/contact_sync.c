/* SPDX-License-Identifier: GPL-3.0-or-later */
#include "contact_sync.h"

void
signal_contact_sync_init(SignalContactSync *sync)
{
    g_return_if_fail(sync != NULL);

    sync->seen = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    sync->active = FALSE;
}

void
signal_contact_sync_clear(SignalContactSync *sync)
{
    g_return_if_fail(sync != NULL);

    g_clear_pointer(&sync->seen, g_hash_table_unref);
    sync->active = FALSE;
}

void
signal_contact_sync_begin(SignalContactSync *sync)
{
    g_return_if_fail(sync != NULL);
    g_return_if_fail(sync->seen != NULL);

    g_hash_table_remove_all(sync->seen);
    sync->active = TRUE;
}

void
signal_contact_sync_mark(SignalContactSync *sync, const char *identifier)
{
    g_return_if_fail(sync != NULL);

    if (!sync->active || identifier == NULL || identifier[0] == '\0')
        return;

    g_hash_table_add(sync->seen, g_strdup(identifier));
}

gboolean
signal_contact_sync_should_remove(const SignalContactSync *sync,
                                  const char *identifier)
{
    g_return_val_if_fail(sync != NULL, FALSE);

    return sync->active && identifier != NULL && identifier[0] != '\0' &&
           !g_hash_table_contains(sync->seen, identifier);
}

void
signal_contact_sync_end(SignalContactSync *sync)
{
    g_return_if_fail(sync != NULL);

    sync->active = FALSE;
    if (sync->seen != NULL)
        g_hash_table_remove_all(sync->seen);
}
