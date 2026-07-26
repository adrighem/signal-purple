/* SPDX-License-Identifier: GPL-3.0-or-later */
#include <fcntl.h>
#include <glib.h>
#include <glib/gstdio.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "attachment_file.h"

#define TEST_SIGNAL_MAX_ATTACHMENT_BYTES (25u * 1024u * 1024u)

static char *test_directory;

static char *
test_path(const char *name)
{
    return g_build_filename(test_directory, name, NULL);
}

static void
test_reads_binary_data(void)
{
    const guint8 expected[] = {0x00, 0x01, 0xff, 0x00, 0x7f};
    g_autofree char *path = test_path("binary");
    g_autoptr(GError) error = NULL;
    g_autoptr(GBytes) bytes = NULL;
    gconstpointer contents;
    gsize size;

    g_assert_true(g_file_set_contents(path, (const char *)expected,
                                      (gssize)sizeof(expected), &error));
    g_assert_no_error(error);
    bytes = signal_read_bounded_file(path, sizeof(expected), &error);
    g_assert_no_error(error);
    g_assert_nonnull(bytes);
    contents = g_bytes_get_data(bytes, &size);
    g_assert_cmpuint(size, ==, sizeof(expected));
    g_assert_cmpint(memcmp(contents, expected, sizeof(expected)), ==, 0);
    g_assert_cmpint(g_remove(path), ==, 0);
}

static void
test_rejects_empty_files(void)
{
    g_autofree char *path = test_path("empty");
    g_autoptr(GError) error = NULL;

    g_assert_true(g_file_set_contents(path, "", 0, &error));
    g_assert_no_error(error);
    g_assert_null(signal_read_bounded_file(path, 16, &error));
    g_assert_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA);
    g_assert_cmpint(g_remove(path), ==, 0);
}

static void
test_enforces_the_exact_limit(void)
{
    const gsize maximum = 4096;
    g_autofree guint8 *contents = g_malloc0(maximum + 1);
    g_autofree char *path = test_path("limit");
    g_autoptr(GError) error = NULL;
    g_autoptr(GBytes) bytes = NULL;

    g_assert_true(g_file_set_contents(path, (const char *)contents,
                                      (gssize)maximum, &error));
    g_assert_no_error(error);
    bytes = signal_read_bounded_file(path, maximum, &error);
    g_assert_no_error(error);
    g_assert_nonnull(bytes);
    g_assert_cmpuint(g_bytes_get_size(bytes), ==, maximum);
    g_clear_pointer(&bytes, g_bytes_unref);

    g_assert_true(g_file_set_contents(path, (const char *)contents,
                                      (gssize)(maximum + 1), &error));
    g_assert_no_error(error);
    g_assert_null(signal_read_bounded_file(path, maximum, &error));
    g_assert_error(error, G_IO_ERROR, G_IO_ERROR_MESSAGE_TOO_LARGE);
    g_assert_cmpint(g_remove(path), ==, 0);
}

static void
test_enforces_the_attachment_limit(void)
{
    g_autofree char *path = test_path("sparse");
    g_autoptr(GError) error = NULL;
    g_autoptr(GBytes) bytes = NULL;
    int descriptor = g_open(path, O_CREAT | O_EXCL | O_WRONLY, 0600);

    g_assert_cmpint(descriptor, >=, 0);
    g_assert_cmpint(
        ftruncate(descriptor, (off_t)TEST_SIGNAL_MAX_ATTACHMENT_BYTES), ==, 0);
    g_assert_cmpint(close(descriptor), ==, 0);

    bytes = signal_read_bounded_file(
        path, TEST_SIGNAL_MAX_ATTACHMENT_BYTES, &error);
    g_assert_no_error(error);
    g_assert_nonnull(bytes);
    g_assert_cmpuint(g_bytes_get_size(bytes), ==,
                     TEST_SIGNAL_MAX_ATTACHMENT_BYTES);
    g_clear_pointer(&bytes, g_bytes_unref);

    descriptor = g_open(path, O_WRONLY, 0);
    g_assert_cmpint(descriptor, >=, 0);
    g_assert_cmpint(ftruncate(
                        descriptor,
                        (off_t)TEST_SIGNAL_MAX_ATTACHMENT_BYTES + (off_t)1),
                    ==, 0);
    g_assert_cmpint(close(descriptor), ==, 0);
    g_assert_null(signal_read_bounded_file(
        path, TEST_SIGNAL_MAX_ATTACHMENT_BYTES, &error));
    g_assert_error(error, G_IO_ERROR, G_IO_ERROR_MESSAGE_TOO_LARGE);
    g_assert_cmpint(g_remove(path), ==, 0);
}

static void
test_enforces_limit_when_metadata_underreports(void)
{
    const char *path = "/proc/self/cmdline";
    g_autoptr(GError) error = NULL;
    struct stat metadata;

    if (g_stat(path, &metadata) != 0 || !S_ISREG(metadata.st_mode) ||
        metadata.st_size != 0) {
        g_test_skip("No zero-sized procfs command-line file");
        return;
    }

    g_assert_null(signal_read_bounded_file(path, 1, &error));
    g_assert_error(error, G_IO_ERROR, G_IO_ERROR_MESSAGE_TOO_LARGE);
}

static void
test_rejects_non_regular_files_without_blocking(void)
{
    g_autofree char *fifo = test_path("fifo");
    g_autoptr(GError) error = NULL;

    g_assert_null(signal_read_bounded_file(test_directory, 16, &error));
    g_assert_error(error, G_IO_ERROR, G_IO_ERROR_NOT_REGULAR_FILE);
    g_clear_error(&error);

    g_assert_cmpint(mkfifo(fifo, 0600), ==, 0);
    g_assert_null(signal_read_bounded_file(fifo, 16, &error));
    g_assert_error(error, G_IO_ERROR, G_IO_ERROR_NOT_REGULAR_FILE);
    g_assert_cmpint(g_remove(fifo), ==, 0);
}

static void
test_reports_missing_files(void)
{
    g_autofree char *path = test_path("missing");
    g_autoptr(GError) error = NULL;

    g_assert_null(signal_read_bounded_file(path, 16, &error));
    g_assert_error(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND);
}

int
main(int argc, char **argv)
{
    g_autoptr(GError) error = NULL;
    int result;

    g_test_init(&argc, &argv, NULL);
    test_directory =
        g_dir_make_tmp("signal-purple-attachment-file-XXXXXX", &error);
    g_assert_no_error(error);
    g_assert_nonnull(test_directory);

    g_test_add_func("/signal/attachment-file/binary", test_reads_binary_data);
    g_test_add_func("/signal/attachment-file/empty", test_rejects_empty_files);
    g_test_add_func("/signal/attachment-file/exact-limit",
                    test_enforces_the_exact_limit);
    g_test_add_func("/signal/attachment-file/attachment-limit",
                    test_enforces_the_attachment_limit);
    g_test_add_func("/signal/attachment-file/metadata-underreports",
                    test_enforces_limit_when_metadata_underreports);
    g_test_add_func("/signal/attachment-file/non-regular",
                    test_rejects_non_regular_files_without_blocking);
    g_test_add_func("/signal/attachment-file/missing",
                    test_reports_missing_files);
    result = g_test_run();

    g_assert_cmpint(g_rmdir(test_directory), ==, 0);
    g_clear_pointer(&test_directory, g_free);
    return result;
}
