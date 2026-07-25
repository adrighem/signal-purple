#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later

import json
import pathlib
import re


PROJECT_ROOT = pathlib.Path(__file__).resolve().parents[1]
LOCK_PATH = pathlib.PurePosixPath("rust/signal-core/Cargo.lock")
MARKER = "# x-release-please-version"
RELEASE_ARTIFACTS_WORKFLOW = pathlib.PurePosixPath(
    ".github/workflows/release-artifacts.yml"
)
RELEASE_PLEASE_WORKFLOW = pathlib.PurePosixPath(
    ".github/workflows/release-please.yml"
)


def fail(message: str) -> None:
    raise SystemExit(message)


def validate_release_config() -> None:
    config = json.loads(
        (PROJECT_ROOT / "release-please-config.json").read_text(encoding="utf-8")
    )
    if config.get("draft") is not True:
        fail("Release Please must create a draft before artifacts are built")
    if config.get("force-tag-creation") is not True:
        fail("Release Please must create the tag for its draft release")

    try:
        package_config = config["packages"]["."]
        extra_files = package_config["extra-files"]
    except (KeyError, TypeError) as error:
        fail(f"release-please extra-files configuration is missing: {error}")
    if config.get("skip-github-release") is True or package_config.get(
        "skip-github-release"
    ) is True:
        fail("Release Please must own GitHub release creation")
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


def require_fragments(
    text: str, path: pathlib.PurePath, fragments: list[str]
) -> None:
    for fragment in fragments:
        if fragment not in text:
            fail(f"{path} is missing the release contract fragment: {fragment}")


def reject_fragments(
    text: str, path: pathlib.PurePath, fragments: list[str]
) -> None:
    for fragment in fragments:
        if fragment in text:
            fail(f"{path} retains obsolete release behavior: {fragment}")


def validate_release_workflows() -> None:
    release_please = (PROJECT_ROOT / RELEASE_PLEASE_WORKFLOW).read_text(
        encoding="utf-8"
    )
    require_fragments(
        release_please,
        RELEASE_PLEASE_WORKFLOW,
        [
            "id: app-token",
            "actions/create-github-app-token@"
            "bcd2ba49218906704ab6c1aa796996da409d3eb1",
            "client-id: ${{ vars.RELEASE_PLEASE_APP_CLIENT_ID }}",
            "private-key: ${{ secrets.RELEASE_PLEASE_APP_PRIVATE_KEY }}",
            "permission-contents: write",
            "permission-pull-requests: write",
            "id: release",
            "token: ${{ steps.app-token.outputs.token }}",
            "release_created: ${{ steps.release.outputs.release_created }}",
            "release_sha: ${{ steps.release.outputs.sha }}",
            "release_tag: ${{ steps.release.outputs.tag_name }}",
            "release_version: ${{ steps.release.outputs.version }}",
            "if: ${{ needs.release-please.outputs.release_created == 'true' }}",
            "uses: ./.github/workflows/release-artifacts.yml",
        ],
    )
    reject_fragments(
        release_please,
        RELEASE_PLEASE_WORKFLOW,
        [
            "workflow_dispatch",
            "RELEASE_PLEASE_TOKEN",
            "secrets.GITHUB_TOKEN",
        ],
    )

    release_artifacts = (PROJECT_ROOT / RELEASE_ARTIFACTS_WORKFLOW).read_text(
        encoding="utf-8"
    )
    require_fragments(
        release_artifacts,
        RELEASE_ARTIFACTS_WORKFLOW,
        [
            "workflow_call:",
            "RELEASE_SHA: ${{ inputs.release_sha }}",
            "RELEASE_TAG: ${{ inputs.release_tag }}",
            "RELEASE_VERSION: ${{ inputs.release_version }}",
            'test "$GITHUB_REF" = refs/heads/main',
            'test "$GITHUB_SHA" = "$RELEASE_SHA"',
            'test "$commit" = "$RELEASE_SHA"',
            "gh api --paginate",
            "cannot add missing asset to published release",
            "-F draft=false",
            "-F prerelease=true",
            "-f make_latest=false",
        ],
    )
    reject_fragments(
        release_artifacts,
        RELEASE_ARTIFACTS_WORKFLOW,
        [
            "repository_dispatch",
            "types: [published]",
            "workflow_dispatch",
            "git verify-tag",
            "RELEASE_KEY_FINGERPRINT",
            "gh release create",
            "--generate-notes",
        ],
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
    validate_release_workflows()
    validate_lock_marker()


if __name__ == "__main__":
    main()
