#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later

import json
import pathlib
import re


PROJECT_ROOT = pathlib.Path(__file__).resolve().parents[1]
LOCK_PATH = pathlib.PurePosixPath("rust/signal-core/Cargo.lock")
MARKER = "# x-release-please-version"


def fail(message: str) -> None:
    raise SystemExit(message)


def validate_release_config() -> None:
    config = json.loads(
        (PROJECT_ROOT / "release-please-config.json").read_text(encoding="utf-8")
    )
    try:
        extra_files = config["packages"]["."]["extra-files"]
    except (KeyError, TypeError) as error:
        fail(f"release-please extra-files configuration is missing: {error}")
    if not isinstance(extra_files, list):
        fail("release-please extra-files configuration must be a list")

    matching_entries = []
    for entry in extra_files:
        if isinstance(entry, str):
            path = entry
        elif isinstance(entry, dict):
            path = entry.get("path")
        else:
            continue
        if path == LOCK_PATH.as_posix():
            matching_entries.append(entry)

    if matching_entries != [{"type": "generic", "path": LOCK_PATH.as_posix()}]:
        fail(
            "Cargo.lock must appear exactly once as a generic Release Please "
            "extra-file"
        )


def validate_lock_marker() -> None:
    version = (PROJECT_ROOT / "version.txt").read_text(encoding="utf-8").strip()
    lock_text = (PROJECT_ROOT / LOCK_PATH).read_text(encoding="utf-8")

    if lock_text.count(MARKER) != 1:
        fail(f"{MARKER} must occur exactly once in Cargo.lock")

    package_blocks = re.split(r"(?m)^\[\[package\]\]\s*$", lock_text)
    signal_core_blocks = [
        block
        for block in package_blocks
        if re.search(r'(?m)^name = "signal-core"$', block)
    ]
    if len(signal_core_blocks) != 1:
        fail("Cargo.lock must contain exactly one signal-core package")

    expected_line = f'version = "{version}" {MARKER}'
    if signal_core_blocks[0].splitlines().count(expected_line) != 1:
        fail(
            "signal-core's Cargo.lock version must match version.txt and retain "
            f"{MARKER}"
        )


def main() -> None:
    validate_release_config()
    validate_lock_marker()


if __name__ == "__main__":
    main()
