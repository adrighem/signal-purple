#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu

repository=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
manifest="$repository/rust/signal-core/Cargo.toml"
build_directory=${SIGNAL_PURPLE_BUILD_DIR:-"$repository/build"}
build_jobs=${SIGNAL_PURPLE_BUILD_JOBS:-2}
install_prefix=${SIGNAL_PURPLE_INSTALL_PREFIX:-/usr}
c_formatter=${SIGNAL_PURPLE_CLANG_FORMAT:-clang-format-19}
c_format_style='{BasedOnStyle: LLVM, IndentWidth: 4,'
c_format_style="$c_format_style ContinuationIndentWidth: 4, ColumnLimit: 0,"
c_format_style="$c_format_style BreakBeforeBraces: Custom,"
c_format_style="$c_format_style BraceWrapping: {AfterFunction: true},"
c_format_style="$c_format_style AlwaysBreakAfterReturnType: AllDefinitions,"
c_format_style="$c_format_style SortIncludes: Never, Cpp11BracedListStyle: false}"
case " ${CFLAGS-} " in
    *" -Werror "*) ;;
    *) CFLAGS="${CFLAGS:+$CFLAGS }-Werror" ;;
esac
export CFLAGS

section()
{
    printf '\n==> %s\n' "$1"
}

check_c_formatter()
{
    if ! command -v "$c_formatter" >/dev/null 2>&1; then
        printf 'required formatter not found: %s\n' "$c_formatter" >&2
        exit 1
    fi
    c_formatter_version=$("$c_formatter" --version)
    case "$c_formatter_version" in
        *"clang-format version 19."*) ;;
        *)
            printf 'expected clang-format 19: %s\n' \
                "$c_formatter_version" >&2
            exit 1
            ;;
    esac
}

c_format_files()
{
    find "$repository/include" "$repository/src" "$repository/tests" \
        -type f \( -name '*.c' -o -name '*.h' \) \
        -exec "$c_formatter" --style="$c_format_style" "$@" {} +
}

c_format()
{
    section "Checking C formatting"
    check_c_formatter
    c_format_files --dry-run --Werror
}

c_format_fix()
{
    section "Formatting C"
    check_c_formatter
    c_format_files -i
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
    if [ "${SIGNAL_PURPLE_REQUIRE_FFMPEG_TEST-}" = 1 ]; then
        converter_test=backend::tests::installed_ffmpeg_converter_produces_a_bounded_animation
        cargo test --locked --manifest-path "$manifest" -- \
            --skip "$converter_test"
        cargo test --locked --manifest-path "$manifest" "$converter_test" -- \
            --exact --test-threads=1
    else
        cargo test --locked --manifest-path "$manifest"
    fi
}

release_helpers()
{
    section "Testing the validation gate"
    sh "$repository/tests/test_check_gate.sh"
    section "Testing deterministic source archives"
    sh "$repository/tests/test_make_source_archive.sh"
    section "Testing release artifact helpers"
    sh "$repository/tests/test_release_artifacts.sh"
    section "Testing the APT repository"
    "$repository/tests/test_apt_repository.sh"
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
    c_format
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
    printf 'usage: %s [fast|full|c-format|c-format-fix|rust-format|rust-lint|rust-test|release|configure|build|c-test|install-probe]\n' \
        "$0" >&2
    exit 2
fi

case "$mode" in
    fast) fast ;;
    full) full ;;
    c-format) c_format ;;
    c-format-fix) c_format_fix ;;
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
