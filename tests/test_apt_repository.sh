#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
set -euo pipefail

project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
temporary=$(mktemp -d "${TMPDIR:-/tmp}/signal-purple-apt-test.XXXXXX")
gpg_home=$temporary/gnupg
wrong_gpg_home=$temporary/wrong-gnupg
cleanup()
{
    for test_gpg_directory in "$gpg_home" "$wrong_gpg_home"; do
        if [ -d "$test_gpg_directory" ]; then
            GNUPGHOME=$test_gpg_directory gpgconf --kill all \
                >/dev/null 2>&1 || true
        fi
    done
    rm -rf -- "$temporary"
}
trap cleanup EXIT HUP INT TERM

for command in apt-cache apt-ftparchive apt-get dpkg-deb gpg gpgv python3; do
    command -v "$command" >/dev/null 2>&1 || {
        printf 'required test command not found: %s\n' "$command" >&2
        exit 1
    }
done

releases_json=$temporary/releases.json
selected_assets=$temporary/selected-assets.tsv
cat > "$releases_json" <<'JSON'
[
  {
    "id": 40,
    "tag_name": "v9.0.0",
    "draft": false,
    "prerelease": true,
    "published_at": "2026-08-04T00:00:00Z",
    "assets": []
  },
  {
    "id": 35,
    "tag_name": "v1.0.0",
    "draft": false,
    "prerelease": false,
    "published_at": "2026-08-05T00:00:00Z",
    "assets": []
  },
  {
    "id": 30,
    "tag_name": "v1.2.3",
    "draft": false,
    "prerelease": false,
    "published_at": "2026-08-03T00:00:00Z",
    "assets": [
      {
        "id": 301,
        "name": "signal-purple_1.2.3-1_amd64.deb",
        "digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "size": 10,
        "state": "uploaded"
      },
      {
        "id": 302,
        "name": "signal-purple-dbgsym_1.2.3-1_amd64.deb",
        "digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "size": 20,
        "state": "uploaded"
      }
    ]
  },
  {
    "id": 20,
    "tag_name": "v1.2.2",
    "draft": false,
    "prerelease": false,
    "published_at": "2026-08-02T00:00:00Z",
    "assets": [
      {
        "id": 201,
        "name": "signal-purple_1.2.2-1_amd64.deb",
        "digest": "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "size": 10,
        "state": "uploaded"
      },
      {
        "id": 202,
        "name": "signal-purple-dbgsym_1.2.2-1_amd64.deb",
        "digest": "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        "size": 20,
        "state": "uploaded"
      }
    ]
  },
  {
    "id": 10,
    "tag_name": "v1.2.1",
    "draft": true,
    "prerelease": false,
    "published_at": "2026-08-01T00:00:00Z",
    "assets": []
  }
]
JSON

"$project_root/scripts/select-apt-release-assets.py" "$releases_json" \
    > "$selected_assets"
test "$(wc -l < "$selected_assets")" -eq 4
test "$(awk -F '\t' 'NR == 1 { print $1 }' "$selected_assets")" = v1.2.3
test "$(awk -F '\t' 'NR == 3 { print $1 }' "$selected_assets")" = v1.2.2
test "$(awk -F '\t' 'NR == 1 { print $2 }' "$selected_assets")" \
    = signal-purple_1.2.3-1_amd64.deb
test "$(awk -F '\t' 'NR == 4 { print $3 }' "$selected_assets")" -eq 202

bad_releases=$temporary/bad-releases.json
python3 - "$releases_json" "$bad_releases" <<'PY'
import json
import pathlib
import sys

releases = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
next(release for release in releases if release["tag_name"] == "v1.2.3")[
    "assets"
].pop()
pathlib.Path(sys.argv[2]).write_text(json.dumps(releases), encoding="utf-8")
PY
if "$project_root/scripts/select-apt-release-assets.py" "$bad_releases" \
    > /dev/null 2>&1; then
    printf '%s\n' 'incomplete stable release unexpectedly selected' >&2
    exit 1
fi

package_directory=$temporary/packages
mkdir "$package_directory"
make_package()
{
    package=$1
    version=$2
    package_root=$temporary/package-$package-$version
    mkdir -p "$package_root/DEBIAN" "$package_root/usr/share/doc/$package"
    printf '%s\n' \
        "Package: $package" \
        "Version: $version" \
        'Architecture: amd64' \
        'Maintainer: signal-purple test <test@example.invalid>' \
        'Section: net' \
        'Priority: optional' \
        "Description: test package $package $version" \
        > "$package_root/DEBIAN/control"
    printf '%s\n' "$package $version" \
        > "$package_root/usr/share/doc/$package/README"
    dpkg-deb --root-owner-group --build "$package_root" \
        "$package_directory/${package}_${version}_amd64.deb" >/dev/null
}

for version in 1.2.2-1 1.2.3-1; do
    make_package signal-purple "$version"
    make_package signal-purple-dbgsym "$version"
done

repository=$temporary/site/apt
"$project_root/scripts/build-apt-repository.sh" \
    "$package_directory" "$repository" debian-13

packages_file=$repository/dists/debian-13/main/binary-amd64/Packages
packages_gzip=$packages_file.gz
release_file=$repository/dists/debian-13/Release
test "$(grep -c '^Package: ' "$packages_file")" -eq 4
grep -Fx 'Package: signal-purple' "$packages_file" >/dev/null
grep -Fx 'Package: signal-purple-dbgsym' "$packages_file" >/dev/null
grep -Fx 'Version: 1.2.2-1' "$packages_file" >/dev/null
grep -Fx 'Version: 1.2.3-1' "$packages_file" >/dev/null
grep -Fx 'Architecture: amd64' "$packages_file" >/dev/null
grep -Fx \
    'Filename: pool/main/s/signal-purple/signal-purple_1.2.3-1_amd64.deb' \
    "$packages_file" >/dev/null
test "$(grep -c '^SHA256: ' "$packages_file")" -eq 4
gzip --decompress --stdout "$packages_gzip" | cmp - "$packages_file"

by_hash=$repository/dists/debian-13/main/binary-amd64/by-hash/SHA256
for index in "$packages_file" "$packages_gzip"; do
    digest=$(sha256sum "$index")
    digest=${digest%% *}
    cmp "$index" "$by_hash/$digest"
done

grep -Fx 'Acquire-By-Hash: yes' "$release_file" >/dev/null
grep -Fx 'Architectures: amd64' "$release_file" >/dev/null
grep -Fx 'Codename: debian-13' "$release_file" >/dev/null
grep -Fx 'Components: main' "$release_file" >/dev/null
grep -Fx 'Suite: debian-13' "$release_file" >/dev/null
if grep -q '^Valid-Until:' "$release_file"; then
    printf '%s\n' 'quiet-project repository unexpectedly expires' >&2
    exit 1
fi

mkdir -m 0700 "$gpg_home"
export GNUPGHOME=$gpg_home
gpg --batch --quiet --pinentry-mode loopback --passphrase '' \
    --quick-generate-key \
    'signal-purple APT test <apt-test@example.invalid>' rsa2048 sign 1d
fingerprint=$(gpg --batch --with-colons --list-secret-keys \
    | awk -F: '$1 == "fpr" { print $10; exit }')
keyring=$repository/signal-purple-archive-keyring.gpg
inrelease=$repository/dists/debian-13/InRelease
release_signature=$repository/dists/debian-13/Release.gpg
gpg --batch --export-options export-minimal --export "$fingerprint" > "$keyring"

sign_release()
{
    output=$1
    shift
    printf '\n' | gpg --batch --yes --pinentry-mode loopback \
        --passphrase-fd 0 --local-user "$fingerprint" --digest-algo SHA512 \
        --output "$output" "$@" "$release_file"
}
sign_release "$inrelease" --clearsign
sign_release "$release_signature" --armor --detach-sign
gpgv --keyring "$keyring" "$inrelease" >/dev/null
gpgv --keyring "$keyring" "$release_signature" "$release_file" >/dev/null

mkdir -m 0700 "$wrong_gpg_home"
GNUPGHOME=$wrong_gpg_home gpg --batch --quiet --passphrase '' \
    --quick-generate-key \
    'wrong signal-purple APT test <wrong@example.invalid>' rsa2048 sign 1d
wrong_keyring=$temporary/wrong-keyring.gpg
GNUPGHOME=$wrong_gpg_home gpg --batch --quiet --export > "$wrong_keyring"
if gpgv --keyring "$wrong_keyring" "$inrelease" >/dev/null 2>&1; then
    printf '%s\n' 'APT metadata unexpectedly verified with an unrelated key' >&2
    exit 1
fi
tampered_release=$temporary/tampered-Release
cp -- "$release_file" "$tampered_release"
printf '%s\n' tampered >> "$tampered_release"
if gpgv --keyring "$keyring" "$release_signature" "$tampered_release" \
    > /dev/null 2>&1; then
    printf '%s\n' 'tampered APT Release metadata unexpectedly verified' >&2
    exit 1
fi

apt_root=$temporary/apt-root
apt_home=$temporary/apt-home
mkdir -p \
    "$apt_root/etc/apt/apt.conf.d" \
    "$apt_root/etc/apt/preferences.d" \
    "$apt_root/etc/apt/sources.list.d" \
    "$apt_root/var/cache/apt/archives/partial" \
    "$apt_root/var/lib/apt/lists/partial" \
    "$apt_root/var/lib/dpkg" \
    "$apt_home"
: > "$apt_root/etc/apt/sources.list"
: > "$apt_root/var/lib/dpkg/status"
cat > "$apt_root/etc/apt/sources.list.d/signal-purple.sources" <<EOF
Types: deb
URIs: file:$repository
Suites: debian-13
Components: main
Architectures: amd64
Signed-By: $keyring
EOF

apt_options=(
    -o "Dir=$apt_root"
    -o "Dir::Etc=$apt_root/etc/apt"
    -o Dir::Etc::main=apt.conf
    -o Dir::Etc::parts=apt.conf.d
    -o Dir::Etc::sourcelist=sources.list
    -o Dir::Etc::sourceparts=sources.list.d
    -o "Dir::State=$apt_root/var/lib/apt"
    -o "Dir::State::status=$apt_root/var/lib/dpkg/status"
    -o "Dir::Cache=$apt_root/var/cache/apt"
    -o Debug::NoLocking=true
    -o "APT::Sandbox::User=$(id -un)"
)
apt_output=$temporary/apt-update.log
if ! env -i PATH=/usr/bin:/bin HOME="$apt_home" \
    apt-get "${apt_options[@]}" update > "$apt_output" 2>&1; then
    tail -30 "$apt_output" >&2
    exit 1
fi
policy=$(env -i PATH=/usr/bin:/bin HOME="$apt_home" \
    apt-cache "${apt_options[@]}" policy signal-purple)
printf '%s\n' "$policy" | grep -F 'Candidate: 1.2.3-1' >/dev/null
printf '%s\n' "$policy" | grep -F '1.2.2-1' >/dev/null

download_directory=$temporary/download
mkdir "$download_directory"
(
    cd "$download_directory"
    env -i PATH=/usr/bin:/bin HOME="$apt_home" \
        apt-get "${apt_options[@]}" download signal-purple=1.2.3-1 \
        > /dev/null 2>&1
)
cmp "$download_directory/signal-purple_1.2.3-1_amd64.deb" \
    "$repository/pool/main/s/signal-purple/signal-purple_1.2.3-1_amd64.deb"

if "$project_root/scripts/build-apt-repository.sh" \
    "$package_directory" "$repository" debian-13 > /dev/null 2>&1; then
    printf '%s\n' 'non-empty APT repository output unexpectedly accepted' >&2
    exit 1
fi
if "$project_root/scripts/build-apt-repository.sh" \
    "$package_directory" "$temporary/ubuntu" ubuntu-24.04 \
    > /dev/null 2>&1; then
    printf '%s\n' 'unsupported APT suite unexpectedly accepted' >&2
    exit 1
fi
