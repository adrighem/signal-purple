#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu

repository=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
manifest="$repository/rust/signal-core/Cargo.toml"
build_directory=${SIGNAL_PURPLE_BUILD_DIR:-"$repository/build"}
build_jobs=${SIGNAL_PURPLE_BUILD_JOBS:-2}
install_prefix=${SIGNAL_PURPLE_INSTALL_PREFIX:-/usr}
case " ${CFLAGS-} " in
    *" -Werror "*) ;;
    *) CFLAGS="${CFLAGS:+$CFLAGS }-Werror" ;;
esac
export CFLAGS

section()
{
    printf '\n==> %s\n' "$1"
}

rust_format()
{
    section "Checking Rust formatting"
    cargo fmt --manifest-path "$manifest" -- --check
}

rust_lint()
{
    section "Linting Rust"
    cargo clippy --locked --manifest-path "$manifest" \
        --all-targets -- -D warnings
}

rust_test()
{
    section "Testing Rust"
    cargo test --locked --manifest-path "$manifest"
}

release_helpers()
{
    section "Testing the validation gate"
    sh "$repository/tests/test_check_gate.sh"
    section "Testing deterministic source archives"
    sh "$repository/tests/test_make_source_archive.sh"
    section "Testing release artifact helpers"
    sh "$repository/tests/test_release_artifacts.sh"
}

configure()
{
    section "Configuring the C adapter and plugin"
    cmake -S "$repository" -B "$build_directory" -G Ninja \
        -DCMAKE_BUILD_TYPE=Debug \
        -DBUILD_TESTING=ON \
        -DCMAKE_INSTALL_PREFIX="$install_prefix"
}

build()
{
    section "Building the C adapter and plugin"
    cmake --build "$build_directory" --parallel "$build_jobs"
}

c_test()
{
    section "Testing the C adapter and plugin"
    ctest --test-dir "$build_directory" --output-on-failure --no-tests=error
}

install_probe()
{
    section "Probing a staged installation"
    "$repository/scripts/probe-staged-install.sh" "$build_directory"
}

fast()
{
    rust_format
    rust_lint
    rust_test
}

full()
{
    fast
    release_helpers
    configure
    build
    c_test
    install_probe
}

mode=${1:-full}
if [ "$#" -gt 1 ]; then
    printf 'usage: %s [fast|full|rust-format|rust-lint|rust-test|release|configure|build|c-test|install-probe]\n' \
        "$0" >&2
    exit 2
fi

case "$mode" in
    fast) fast ;;
    full) full ;;
    rust-format) rust_format ;;
    rust-lint) rust_lint ;;
    rust-test) rust_test ;;
    release) release_helpers ;;
    configure) configure ;;
    build) build ;;
    c-test) c_test ;;
    install-probe) install_probe ;;
    *)
        printf 'unknown check mode: %s\n' "$mode" >&2
        exit 2
        ;;
esac
