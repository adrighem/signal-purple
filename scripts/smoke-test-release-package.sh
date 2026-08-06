#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu

if [ "$#" -lt 2 ] || [ "$#" -gt 4 ]; then
    printf 'usage: %s VERSION ARTIFACT_DIRECTORY [DEBIAN_VERSION [DISTRO_ID]]\n' \
        "$0" >&2
    exit 2
fi

version=$1
artifact_directory=$2
debian_version=${3:-"$version-1"}
distro_id=${4:-debian-13}
case "$distro_id" in
    debian-13 | ubuntu-24.04-lts) ;;
    *)
        printf 'unsupported release distribution: %s\n' "$distro_id" >&2
        exit 1
        ;;
esac
package="$artifact_directory/signal-purple_${debian_version}_${distro_id}_amd64.deb"
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
if [ "$(uname -s)" = Linux ]; then
    readelf -d "$backend" | grep -F NODELETE
fi
readelf -d "$plugin" \
    | grep -F "Library runpath: [\$ORIGIN/signal-purple]"
G_DEBUG=fatal-warnings timeout --signal=TERM --kill-after=5s 30s \
    "$probe" "$plugin"
dpkg -V signal-purple
