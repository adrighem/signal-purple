#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu

repository=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
check_script="$repository/scripts/check.sh"
ci_workflow="$repository/.github/workflows/ci.yml"

require_argument()
{
    subject=$1
    file=$2
    argument=$3
    if ! grep -F -- "$argument" "$file" >/dev/null; then
        printf '%s is missing required argument: %s\n' \
            "$subject" "$argument" >&2
        exit 1
    fi
}

require_argument "check.sh" "$check_script" "-DBUILD_TESTING=ON"
require_argument "check.sh" "$check_script" "--no-tests=error"
require_argument "check.sh" "$check_script" "clang-format-19"
require_argument "check.sh" "$check_script" "--dry-run"
require_argument "check.sh" "$check_script" "c-format-fix) c_format_fix"
require_argument "Debian CI" "$ci_workflow" "clang-format-19"
require_argument "Debian CI" "$ci_workflow" "scripts/check.sh c-format"
require_argument "Debian CI" "$ci_workflow" \
    "cargo test --locked --manifest-path rust/signal-core/Cargo.toml"
require_argument "Debian CI" "$ci_workflow" "-DBUILD_TESTING=ON"
require_argument "Debian CI" "$ci_workflow" "--no-tests=error"
