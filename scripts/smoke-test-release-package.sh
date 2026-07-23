#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
    printf 'usage: %s VERSION ARTIFACT_DIRECTORY [DEBIAN_VERSION]\n' \
        "$0" >&2
    exit 2
fi

version=$1
artifact_directory=$2
debian_version=${3:-"$version-1"}
package="$artifact_directory/signal-purple_${debian_version}_amd64.deb"
probe="$artifact_directory/.validation/plugin-probe"

test -s "$package"
test -x "$probe"
dpkg --install "$package"
test "$(dpkg-query -W -f='${Version}' signal-purple)" = "$debian_version"

plugin_directory=$(pkgconf --variable=plugindir purple)
plugin="$plugin_directory/libsignal-purple.so"
backend="$plugin_directory/signal-purple/libsignal_core.so"
test -r "$plugin"
test -r "$backend"
test "$(ldd "$plugin" \
    | awk '$1 == "libsignal_core.so" { print $3; exit }')" = "$backend"
readelf -d "$plugin" \
    | grep -F "Library runpath: [\$ORIGIN/signal-purple]"
G_DEBUG=fatal-warnings "$probe" "$plugin"
dpkg -V signal-purple
