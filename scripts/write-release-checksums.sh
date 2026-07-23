#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu

if [ "$#" -ne 2 ]; then
    printf 'usage: %s VERSION ARTIFACT_DIRECTORY\n' "$0" >&2
    exit 2
fi

version=$1
artifact_directory=$2
architecture=$(dpkg-architecture -qDEB_HOST_ARCH)

if ! printf '%s\n' "$version" \
    | grep -Eq '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'; then
    printf 'invalid release version: %s\n' "$version" >&2
    exit 1
fi
if [ "$architecture" != amd64 ]; then
    printf 'release artifacts support amd64, not %s\n' "$architecture" >&2
    exit 1
fi
if [ ! -d "$artifact_directory" ]; then
    printf 'artifact directory does not exist: %s\n' \
        "$artifact_directory" >&2
    exit 1
fi

source_archive="signal-purple_${version}.orig.tar.xz"
runtime="signal-purple_${version}-1_${architecture}.deb"
debug="signal-purple-dbgsym_${version}-1_${architecture}.deb"
sbom="signal-purple-${version}.spdx.json"

for artifact in "$source_archive" "$runtime" "$debug" "$sbom"; do
    if [ ! -s "$artifact_directory/$artifact" ] \
        || [ -L "$artifact_directory/$artifact" ]; then
        printf 'missing or unsafe release artifact: %s\n' "$artifact" >&2
        exit 1
    fi
done

(
    cd "$artifact_directory"
    sha256sum "$source_archive" "$runtime" "$debug" "$sbom" \
        > SHA256SUMS
    sha256sum --check SHA256SUMS
)
