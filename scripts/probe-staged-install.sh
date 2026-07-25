#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu

if [ "$#" -ne 1 ]; then
    printf 'usage: %s BUILD_DIRECTORY\n' "$0" >&2
    exit 2
fi

build_directory=$1
temporary=$(mktemp -d "${TMPDIR:-/tmp}/signal-purple-install.XXXXXX")
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
stage="$temporary/stage"

cmake --install "$build_directory" --prefix "$stage"
plugin=$(awk '/libsignal-purple\.so$/ { print; exit }' \
    "$build_directory/install_manifest.txt")
case "$plugin" in
    "$stage"/*) ;;
    *)
        printf 'installed plugin escaped staging prefix: %s\n' "$plugin" >&2
        exit 1
        ;;
esac

plugin_directory=$(dirname -- "$plugin")
backend="$plugin_directory/signal-purple/libsignal_core.so"
probe="$build_directory/plugin-probe"
test -r "$plugin"
test -r "$backend"
test -x "$probe"

ldd "$plugin"
resolved_backend=$(ldd "$plugin" \
    | sed -n 's/^[[:space:]]*libsignal_core\.so => \(.*\) (0x[0-9a-fA-F]*)$/\1/p')
test "$resolved_backend" = "$backend"
G_DEBUG=fatal-warnings "$probe" "$plugin"
