#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu

if [ "$#" -ne 2 ] && [ "$#" -ne 3 ] && [ "$#" -ne 5 ]; then
    printf '%s\n' \
        "usage: $0 REVISION OUTPUT_DIRECTORY [DEBIAN_VERSION]" \
        "       $0 REVISION OUTPUT_DIRECTORY DEBIAN_VERSION ARCHIVE_A ARCHIVE_B" \
        >&2
    exit 2
fi

revision=$1
output=$2
argument_count=$#
repository=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
commit=$(git -C "$repository" rev-parse --verify --end-of-options \
    "$revision^{commit}")
version=$(git -C "$repository" show "$commit:version.txt")
debian_version=${3:-"$version-1"}
architecture=$(dpkg-architecture -qDEB_HOST_ARCH)
build_profile=pkg.signal-purple.upstream-rust

if ! printf '%s\n' "$version" \
    | grep -Eq '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'; then
    printf 'invalid upstream version: %s\n' "$version" >&2
    exit 1
fi
if ! dpkg --validate-version "$debian_version"; then
    printf 'invalid Debian version: %s\n' "$debian_version" >&2
    exit 1
fi
case "$debian_version" in
    "$version"-* | "$version"+*) ;;
    *)
        printf 'Debian version %s does not belong to upstream %s\n' \
            "$debian_version" "$version" >&2
        exit 1
        ;;
esac
case "$debian_version" in
    *[!0-9A-Za-z.+~-]*)
        printf 'unsupported character in Debian version: %s\n' \
            "$debian_version" >&2
        exit 1
        ;;
esac
if [ "$architecture" != amd64 ]; then
    printf 'release packages support amd64, not %s\n' "$architecture" >&2
    exit 1
fi

mkdir -p "$output"
output=$(CDPATH='' cd -- "$output" && pwd)
if find "$output" -mindepth 1 -maxdepth 1 -print -quit | grep -q .; then
    printf 'output directory is not empty: %s\n' "$output" >&2
    exit 1
fi

temporary=$(mktemp -d "${TMPDIR:-/tmp}/signal-purple-release.XXXXXX")
trap 'rm -rf "$temporary"' EXIT HUP INT TERM

archive_name="signal-purple_${version}.orig.tar.xz"
runtime_name="signal-purple_${debian_version}_${architecture}.deb"
debug_name="signal-purple-dbgsym_${debian_version}_${architecture}.deb"
if [ "$argument_count" -eq 5 ]; then
    archive_a=$4
    archive_b=$5
    test -s "$archive_a"
    test -s "$archive_b"
else
    archive_a="$temporary/source-a/$archive_name"
    archive_b="$temporary/source-b/$archive_name"
    mkdir -p "$(dirname -- "$archive_a")" "$(dirname -- "$archive_b")"
    TMPDIR="$temporary/source-a" \
        "$repository/scripts/make-source-archive.sh" "$commit" "$archive_a"
    TMPDIR="$temporary/source-b" \
        "$repository/scripts/make-source-archive.sh" "$commit" "$archive_b"
fi
xz --test "$archive_a" "$archive_b"
cmp "$archive_a" "$archive_b"

build_package()
{
    label=$1
    archive=$2
    build_root="$temporary/build-$label"
    source_dir="$build_root/signal-purple-$version"
    mkdir -p "$build_root"
    tar -xJf "$archive" -C "$build_root"

    if [ "$debian_version" != "$version-1" ]; then
        sed -i "1s|([^)]*)|($debian_version)|" \
            "$source_dir/debian/changelog"
    fi

    test "$(dpkg-parsechangelog -l"$source_dir/debian/changelog" \
        -S Version)" = "$debian_version"
    test -s "$source_dir/.cargo/config.toml"
    test -d "$source_dir/vendor"
    grep -Fq \
        'cargo (>= 1.94) <!pkg.signal-purple.upstream-rust>,' \
        "$source_dir/debian/control"
    grep -Fq \
        'rustc (>= 1.94) <!pkg.signal-purple.upstream-rust>' \
        "$source_dir/debian/control"

    (
        cd "$source_dir"
        dpkg-checkbuilddeps -P"$build_profile"
        CARGO_BUILD_JOBS=2 \
        CARGO_NET_OFFLINE=true \
        DEB_BUILD_OPTIONS=parallel=2 \
        LC_ALL=C.UTF-8 \
        TZ=UTC \
            ionice -c 3 nice dpkg-buildpackage \
                --build=binary \
                --build-profiles="$build_profile" \
                --jobs=2 \
                --no-sign
    )

    runtime="$build_root/$runtime_name"
    debug="$build_root/$debug_name"
    test -s "$runtime"
    test -s "$debug"
    test "$(dpkg-deb --field "$runtime" Package)" = signal-purple
    test "$(dpkg-deb --field "$debug" Package)" = signal-purple-dbgsym
    test "$(dpkg-deb --field "$runtime" Version)" = "$debian_version"
    test "$(dpkg-deb --field "$debug" Version)" = "$debian_version"
    test "$(dpkg-deb --field "$runtime" Architecture)" = "$architecture"
    test "$(dpkg-deb --field "$debug" Architecture)" = "$architecture"
}

build_package a "$archive_a"
build_package b "$archive_b"
cmp "$temporary/build-a/$runtime_name" "$temporary/build-b/$runtime_name"
cmp "$temporary/build-a/$debug_name" "$temporary/build-b/$debug_name"

cp "$archive_a" "$output/$archive_name"
cp "$temporary/build-a/$runtime_name" "$output/$runtime_name"
cp "$temporary/build-a/$debug_name" "$output/$debug_name"

probe=$(find "$temporary/build-a/signal-purple-$version" \
    -type f -name plugin-probe -perm -u+x -print -quit)
test -n "$probe"
mkdir "$output/.validation"
cp "$probe" "$output/.validation/plugin-probe"

sha256sum "$output/$archive_name" "$output/$runtime_name" \
    "$output/$debug_name"
