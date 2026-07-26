#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu

repository=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
check_script="$repository/scripts/check.sh"

require_argument()
{
    argument=$1
    if ! grep -F -- "$argument" "$check_script" >/dev/null; then
        printf 'check.sh is missing required argument: %s\n' "$argument" >&2
        exit 1
    fi
}

require_argument "-DBUILD_TESTING=ON"
require_argument "--no-tests=error"
