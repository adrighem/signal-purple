#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
temporary=$(mktemp -d "${TMPDIR:-/tmp}/signal-purple-archive-test.XXXXXX")
trap 'rm -rf "$temporary"' EXIT HUP INT TERM

fixture="$temporary/repository"
fake_bin="$temporary/bin"
mkdir -p "$fixture/debian" "$fixture/rust/signal-core" \
    "$fixture/scripts" "$fake_bin"

cp "$project_root/scripts/make-source-archive.sh" "$fixture/scripts/"

cat > "$fixture/version.txt" <<'EOF'
0.2.0
EOF
cat > "$fixture/debian/changelog" <<'EOF'
signal-purple (0.2.0-1) unstable; urgency=medium

  * Test fixture.

 -- Signal Purple Maintainers <maintainers@example.invalid>  Thu, 01 Jan 2026 00:00:00 +0000
EOF
cat > "$fixture/debian/copyright" <<'EOF'
Test fixture
EOF
cat > "$fixture/rust/signal-core/Cargo.toml" <<'EOF'
[package]
name = "signal-core"
version = "0.2.0"
EOF
cat > "$fixture/rust/signal-core/Cargo.lock" <<'EOF'
version = 3
EOF
cat > "$fixture/scripts/generate-vendor-copyright.py" <<'EOF'
#!/usr/bin/env python3
from pathlib import Path
import sys

Path(sys.argv[2]).write_text("Generated fixture copyright\n")
EOF
cat > "$fixture/payload.txt" <<'EOF'
source archive fixture
EOF

cat > "$fake_bin/cargo" <<'EOF'
#!/bin/sh
set -eu

case "$1" in
    vendor)
        mkdir -p vendor/fake-crate-1.0.0
        printf '%s\n' '[package]' 'name = "fake-crate"' 'version = "1.0.0"' \
            > vendor/fake-crate-1.0.0/Cargo.toml
        printf '%s\n' '[source.crates-io]' 'replace-with = "vendored-sources"'
        ;;
    metadata)
        printf '%s\n' '{"packages":[]}'
        ;;
    *)
        printf 'unexpected cargo command: %s\n' "$1" >&2
        exit 1
        ;;
esac
EOF
chmod +x "$fake_bin/cargo" "$fixture/scripts/make-source-archive.sh" \
    "$fixture/scripts/generate-vendor-copyright.py"

git -C "$fixture" init --quiet
git -C "$fixture" config user.name "Archive Test"
git -C "$fixture" config user.email "archive-test@example.invalid"
git -C "$fixture" config commit.gpgSign false
git -C "$fixture" config tag.gpgSign false
git -C "$fixture" config core.autocrlf false
git -C "$fixture" add .
GIT_AUTHOR_DATE="2026-01-01T00:00:00Z" \
GIT_COMMITTER_DATE="2026-01-01T00:00:00Z" \
    git -C "$fixture" commit --quiet --message "fixture"
commit=$(git -C "$fixture" rev-parse HEAD)
GIT_COMMITTER_DATE="2026-01-02T00:00:00Z" \
    git -C "$fixture" tag --annotate --message "release" v0.2.0 "$commit"

commit_archive="$temporary/from-commit.tar.xz"
tag_archive="$temporary/from-tag.tar.xz"
repeated_tag_archive="$temporary/from-tag-again.tar.xz"
option_archive="$temporary/from-option.tar.xz"
PATH="$fake_bin:$PATH" \
    "$fixture/scripts/make-source-archive.sh" "$commit" "$commit_archive" \
    > /dev/null
PATH="$fake_bin:$PATH" \
    "$fixture/scripts/make-source-archive.sh" v0.2.0 "$tag_archive" \
    > /dev/null
PATH="$fake_bin:$PATH" \
    "$fixture/scripts/make-source-archive.sh" v0.2.0 "$repeated_tag_archive" \
    > /dev/null
if PATH="$fake_bin:$PATH" \
    "$fixture/scripts/make-source-archive.sh" --help "$option_archive" \
    > /dev/null 2>&1; then
    printf '%s\n' 'option-like revision unexpectedly succeeded' >&2
    exit 1
fi
test ! -e "$option_archive"

xz --test "$commit_archive" "$tag_archive" "$repeated_tag_archive"
cmp "$commit_archive" "$tag_archive"
cmp "$tag_archive" "$repeated_tag_archive"

archive_root="$temporary/extracted"
mkdir "$archive_root"
tar -xJf "$tag_archive" -C "$archive_root"
test "$(cat "$archive_root/signal-purple-0.2.0/payload.txt")" = \
    "source archive fixture"
test "$(stat -c %Y "$archive_root/signal-purple-0.2.0/payload.txt")" = \
    "$(git -C "$fixture" show -s --format=%ct "$commit")"
