#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
set -euo pipefail

fail()
{
    printf '%s\n' "$1" >&2
    exit 1
}

if [ "$#" -ne 2 ]; then
    printf 'usage: %s PACKAGE_DIRECTORY OUTPUT_DIRECTORY\n' \
        "$(basename -- "$0")" >&2
    exit 2
fi

package_directory=$1
output_directory=$2
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

shopt -s nullglob
package_paths=("$package_directory"/*.deb)
if [ "${#package_paths[@]}" -eq 0 ]; then
    fail "package directory contains no Debian packages"
fi

declare -A package_versions=()
declare -A release_versions=()
declare -A suite_versions=()
declare -A suite_package_counts=(
    [debian-13]=0
    [ubuntu-24.04]=0
)
for source_path in "${package_paths[@]}"; do
    test -f "$source_path" && test ! -L "$source_path" \
        || fail "package input is not a regular file"

    package=$(dpkg-deb --field "$source_path" Package)
    version=$(dpkg-deb --field "$source_path" Version)
    architecture=$(dpkg-deb --field "$source_path" Architecture)
    actual_name=$(basename -- "$source_path")
    case "$package" in
        signal-purple | signal-purple-dbgsym) ;;
        *) fail "unexpected Debian package name: $package" ;;
    esac
    [[ "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)-1$ ]] \
        || fail "unexpected Debian package version: $version"
    test "$architecture" = amd64 \
        || fail "unexpected Debian package architecture: $architecture"

    case "$actual_name" in
        *_debian-13_amd64.deb)
            suite=debian-13
            distro_id=debian-13
            ;;
        *_ubuntu-24.04-lts_amd64.deb)
            suite=ubuntu-24.04
            distro_id=ubuntu-24.04-lts
            ;;
        signal-purple_*_amd64.deb | signal-purple-dbgsym_*_amd64.deb)
            suite=debian-13
            distro_id=
            ;;
        *)
            fail "unsupported Debian release asset name: $actual_name"
            ;;
    esac
    if [ -n "$distro_id" ]; then
        expected_name=${package}_${version}_${distro_id}_${architecture}.deb
    else
        expected_name=${package}_${version}_${architecture}.deb
    fi
    test "$actual_name" = "$expected_name" \
        || fail "Debian package filename does not match its metadata: $actual_name"

    identity=$suite:$package:$version:$architecture
    test -z "${package_versions[$identity]+present}" \
        || fail "duplicate Debian package identity: $identity"
    package_versions[$identity]=present
    release_versions[$version]=present
    suite_versions[$suite:$version]=present
    suite_package_counts[$suite]=$((suite_package_counts[$suite] + 1))
    pool_directory=$output_directory/pool/$suite/main/s/signal-purple
    mkdir -p -- "$pool_directory"
    install -m 0644 -- "$source_path" "$pool_directory/$actual_name"
done

if [ "${#release_versions[@]}" -gt 2 ]; then
    fail "APT repository input contains more than two release versions"
fi
for suite_version in "${!suite_versions[@]}"; do
    suite=${suite_version%%:*}
    version=${suite_version#*:}
    for package in signal-purple signal-purple-dbgsym; do
        identity=$suite:$package:$version:amd64
        test -n "${package_versions[$identity]+present}" \
            || fail "APT repository input lacks $package version $version for $suite"
    done
done

for suite in debian-13 ubuntu-24.04; do
    test "${suite_package_counts[$suite]}" -gt 0 \
        || fail "APT repository input lacks packages for $suite"
    case "$suite" in
        debian-13)
            release_version=13
            description='signal-purple packages for Debian 13'
            ;;
        ubuntu-24.04)
            release_version=24.04
            description='signal-purple packages for Ubuntu 24.04 LTS'
            ;;
    esac

    pool_relative=pool/$suite/main/s/signal-purple
    index_relative=dists/$suite/main/binary-amd64
    index_directory=$output_directory/$index_relative
    mkdir -p -- "$index_directory"
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
            -o APT::FTPArchive::Release::Version="$release_version" \
            -o APT::FTPArchive::Release::Architectures=amd64 \
            -o APT::FTPArchive::Release::Components=main \
            -o APT::FTPArchive::Release::Acquire-By-Hash=yes \
            -o APT::FTPArchive::Release::Description="$description" \
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

    sources_file=$output_directory/signal-purple-$suite.sources
    printf '%s\n' \
        'Types: deb' \
        'URIs: https://adrighem.github.io/signal-purple/apt' \
        "Suites: $suite" \
        'Components: main' \
        'Architectures: amd64' \
        'Signed-By: /etc/apt/keyrings/signal-purple-archive-keyring.gpg' \
        > "$sources_file"
done
