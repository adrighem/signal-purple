#!/bin/sh
set -eu

revision=${1:-HEAD}
repository=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
version=$(git -C "$repository" show "$revision:version.txt")
output=${2:-"$repository/dist/signal-purple_${version}.orig.tar.xz"}
epoch=$(git -C "$repository" show -s --format=%ct "$revision")
temporary=$(mktemp -d "${TMPDIR:-/tmp}/signal-purple-source.XXXXXX")
trap 'rm -rf "$temporary"' EXIT HUP INT TERM

prefix="signal-purple-$version"
source_dir="$temporary/$prefix"
mkdir -p "$source_dir" "$(dirname -- "$output")"
git -C "$repository" archive "$revision" | tar -x -C "$source_dir"
sed -i "1s/([^)]*)/($version-1)/" "$source_dir/debian/changelog"

mkdir -p "$source_dir/.cargo"
(
    cd "$source_dir"
    cargo vendor --locked --versioned-dirs vendor \
        --manifest-path rust/signal-core/Cargo.toml \
        > .cargo/config.toml 2> "$temporary/cargo-vendor.log"
    test -s .cargo/config.toml
    mkdir "$temporary/cargo-home"
    CARGO_HOME="$temporary/cargo-home" CARGO_NET_OFFLINE=true \
        cargo metadata --frozen --no-deps \
        --manifest-path rust/signal-core/Cargo.toml > /dev/null
)

find "$source_dir" -exec touch --no-dereference --date="@$epoch" {} +
tar --sort=name --format=gnu --mtime="@$epoch" --owner=0 --group=0 \
    --numeric-owner -C "$temporary" -cf - "$prefix" \
    | xz --threads=1 --check=crc64 -6 > "$output"

sha256sum "$output"
