/* SPDX-License-Identifier: GPL-3.0-or-later */
#include "signal_purple.h"

#include <errno.h>
#include <glib/gstdio.h>
#include <libsecret/secret.h>
#include <sys/random.h>
#include <sys/stat.h>

static const SecretSchema signal_secret_schema = {
    .name = "io.github.adrighem.signal-purple",
    .flags = SECRET_SCHEMA_NONE,
    .attributes = {
        {"account", SECRET_SCHEMA_ATTRIBUTE_STRING},
        {"store", SECRET_SCHEMA_ATTRIBUTE_STRING},
        {NULL, 0},
    },
};

static void
signal_clear_secret(void *data, gsize length)
{
    volatile guint8 *bytes = data;

    while (length > 0) {
        *bytes++ = 0;
        length--;
    }
}

static const char *
signal_store_id(PurpleAccount *account)
{
    const char *store_id = purple_account_get_string(account, "store-id", NULL);

    if (store_id == NULL || store_id[0] == '\0') {
        g_autofree char *generated = g_uuid_string_random();
        purple_account_set_string(account, "store-id", generated);
        store_id = purple_account_get_string(account, "store-id", NULL);
    }
    return store_id;
}

char *
signal_plaintext_from_markup(const char *markup)
{
    if (markup == NULL)
        return NULL;

    return purple_markup_strip_html(markup);
}

char *
signal_store_path(PurpleAccount *account, GError **error)
{
    const char *configured;
    const char *store_id;
    g_autofree char *account_hash = NULL;
    g_autofree char *directory = NULL;

    g_return_val_if_fail(account != NULL, NULL);

    configured = purple_account_get_string(account, "store-path", "");
    if (configured != NULL && configured[0] != '\0')
        return g_canonicalize_filename(configured, NULL);

    store_id = signal_store_id(account);
    account_hash = g_compute_checksum_for_string(G_CHECKSUM_SHA256, store_id, -1);
    directory = g_build_filename(purple_user_dir(), "signal-purple", NULL);

    if (g_mkdir_with_parents(directory, 0700) != 0) {
        g_set_error(error, G_FILE_ERROR, (gint)g_file_error_from_errno(errno),
                    "Could not create Signal data directory '%s': %s",
                    directory, g_strerror(errno));
        return NULL;
    }

    if (g_chmod(directory, 0700) != 0) {
        g_set_error(error, G_FILE_ERROR, (gint)g_file_error_from_errno(errno),
                    "Could not secure Signal data directory '%s': %s",
                    directory, g_strerror(errno));
        return NULL;
    }

    return g_strdup_printf("%s%c%s.db3", directory, G_DIR_SEPARATOR,
                           account_hash);
}

static char *
signal_generate_passphrase(GError **error)
{
    guint8 bytes[32];
    gsize offset = 0;
    char *encoded;

    while (offset < sizeof(bytes)) {
        ssize_t count = getrandom(bytes + offset, sizeof(bytes) - offset, 0);

        if (count < 0 && errno == EINTR)
            continue;
        if (count <= 0) {
            signal_clear_secret(bytes, sizeof(bytes));
            g_set_error(error, G_FILE_ERROR,
                        (gint)g_file_error_from_errno(errno),
                        "Could not generate a database encryption key: %s",
                        g_strerror(errno));
            return NULL;
        }
        offset += (gsize)count;
    }

    encoded = g_base64_encode(bytes, sizeof(bytes));
    signal_clear_secret(bytes, sizeof(bytes));
    return encoded;
}

char *
signal_secret_get_or_create(PurpleAccount *account, const char *store_path,
                            GError **error)
{
    const char *username;
    const char *store_id;
    char *password;
    g_autofree char *generated = NULL;
    g_autofree char *label = NULL;

    g_return_val_if_fail(account != NULL, NULL);
    g_return_val_if_fail(store_path != NULL, NULL);

    username = purple_account_get_username(account);
    if (username == NULL)
        username = "signal";
    store_id = signal_store_id(account);

    password = secret_password_lookup_sync(&signal_secret_schema, NULL, error,
                                           "account", store_id,
                                           "store", store_path, NULL);
    if (password != NULL || (error != NULL && *error != NULL))
        return password;

    generated = signal_generate_passphrase(error);
    if (generated == NULL)
        return NULL;

    label = g_strdup_printf("signal-purple database for %s", username);
    if (!secret_password_store_sync(&signal_secret_schema,
                                    SECRET_COLLECTION_DEFAULT,
                                    label, generated, NULL, error,
                                    "account", store_id,
                                    "store", store_path, NULL)) {
        signal_clear_secret(generated, strlen(generated));
        return NULL;
    }

    return g_steal_pointer(&generated);
}
