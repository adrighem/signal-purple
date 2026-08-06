#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
set -euo pipefail

fail()
{
    printf '%s\n' "$1" >&2
    exit 1
}

if [ "$#" -ne 3 ]; then
    printf 'usage: %s PACKAGE_DIRECTORY OUTPUT_DIRECTORY SUITE\n' \
        "$(basename -- "$0")" >&2
    exit 2
fi

package_directory=$1
output_directory=$2
suite=$3

test "$suite" = debian-13 \
    || fail "unsupported APT repository suite: $suite"
test -d "$package_directory" \
    || fail "package directory does not exist"

for command in apt-ftparchive dpkg-deb gzip realpath sha256sum; do
    command -v "$command" >/dev/null 2>&1 \
        || fail "required command not found: $command"
done

package_directory=$(realpath -- "$package_directory")
if [ -e "$output_directory" ]; then
    test -d "$output_directory" \
        || fail "APT repository output is not a directory"
    if find "$output_directory" -mindepth 1 -print -quit | grep -q .; then
        fail "APT repository output directory is not empty"
    fi
else
    mkdir -p -- "$output_directory"
fi
output_directory=$(realpath -- "$output_directory")

case "$output_directory" in
    "$package_directory" | "$package_directory"/*)
        fail "APT repository output must be outside the package directory"
        ;;
esac

pool_relative=pool/main/s/signal-purple
index_relative=dists/$suite/main/binary-amd64
pool_directory=$output_directory/$pool_relative
index_directory=$output_directory/$index_relative
mkdir -p -- "$pool_directory" "$index_directory"

shopt -s nullglob
package_paths=("$package_directory"/*.deb)
if [ "${#package_paths[@]}" -eq 0 ]; then
    fail "package directory contains no Debian packages"
fi

declare -A package_versions=()
declare -A release_versions=()
for source_path in "${package_paths[@]}"; do
    test -f "$source_path" && test ! -L "$source_path" \
        || fail "package input is not a regular file"

    package=$(dpkg-deb --field "$source_path" Package)
    version=$(dpkg-deb --field "$source_path" Version)
    architecture=$(dpkg-deb --field "$source_path" Architecture)
    case "$package" in
        signal-purple | signal-purple-dbgsym) ;;
        *) fail "unexpected Debian package name: $package" ;;
    esac
    [[ "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)-1$ ]] \
        || fail "unexpected Debian package version: $version"
    test "$architecture" = amd64 \
        || fail "unexpected Debian package architecture: $architecture"

    expected_name=${package}_${version}_${architecture}.deb
    actual_name=$(basename -- "$source_path")
    test "$actual_name" = "$expected_name" \
        || fail "Debian package filename does not match its metadata: $actual_name"

    identity=$package:$version:$architecture
    test -z "${package_versions[$identity]+present}" \
        || fail "duplicate Debian package identity: $identity"
    package_versions[$identity]=present
    release_versions[$version]=present
    install -m 0644 -- "$source_path" "$pool_directory/$actual_name"
done

if [ "${#release_versions[@]}" -gt 2 ]; then
    fail "APT repository input contains more than two release versions"
fi
for version in "${!release_versions[@]}"; do
    for package in signal-purple signal-purple-dbgsym; do
        identity=$package:$version:amd64
        test -n "${package_versions[$identity]+present}" \
            || fail "APT repository input lacks $package version $version"
    done
done

packages_file=$index_directory/Packages
(
    cd -- "$output_directory"
    LC_ALL=C apt-ftparchive packages "$pool_relative"
) > "$packages_file"
gzip --no-name --best --stdout "$packages_file" > "$packages_file.gz"

by_hash_directory=$index_directory/by-hash/SHA256
mkdir -p -- "$by_hash_directory"
for index in "$packages_file" "$packages_file.gz"; do
    digest=$(sha256sum "$index")
    digest=${digest%% *}
    install -m 0644 -- "$index" "$by_hash_directory/$digest"
done

release_file=$output_directory/dists/$suite/Release
(
    cd -- "$output_directory"
    LC_ALL=C apt-ftparchive \
        -o APT::FTPArchive::Release::Origin=signal-purple \
        -o APT::FTPArchive::Release::Label=signal-purple \
        -o APT::FTPArchive::Release::Suite="$suite" \
        -o APT::FTPArchive::Release::Codename="$suite" \
        -o APT::FTPArchive::Release::Version=13 \
        -o APT::FTPArchive::Release::Architectures=amd64 \
        -o APT::FTPArchive::Release::Components=main \
        -o APT::FTPArchive::Release::Acquire-By-Hash=yes \
        -o APT::FTPArchive::Release::Description='signal-purple packages for Debian 13' \
        release "dists/$suite"
) > "$release_file"

grep -Fx 'Acquire-By-Hash: yes' "$release_file" >/dev/null \
    || fail "APT Release metadata lacks Acquire-By-Hash"
grep -Fx 'Architectures: amd64' "$release_file" >/dev/null \
    || fail "APT Release metadata has the wrong architecture"
grep -Fx 'Components: main' "$release_file" >/dev/null \
    || fail "APT Release metadata has the wrong component"
grep -Fx "Suite: $suite" "$release_file" >/dev/null \
    || fail "APT Release metadata has the wrong suite"
