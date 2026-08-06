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
APT_REPOSITORY_WORKFLOW = pathlib.PurePosixPath(
    ".github/workflows/apt-repository.yml"
)
RELEASE_PLEASE_WORKFLOW = pathlib.PurePosixPath(
    ".github/workflows/release-please.yml"
)
RELEASE_BUILDER = pathlib.PurePosixPath("scripts/build-release-artifacts.sh")
RELEASE_DOCKERFILE = pathlib.PurePosixPath(".github/release/Dockerfile")
CMAKE_CONFIG = pathlib.PurePosixPath("CMakeLists.txt")


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
            "uses: ./.github/workflows/apt-repository.yml",
            "actions: read",
            "pages: write",
            "id-token: write",
            "cleanup-failed-release:",
            "needs.release-artifacts.result != 'success'",
            "id: cleanup-app-token",
            "GH_TOKEN: ${{ steps.cleanup-app-token.outputs.token }}",
            '"repos/$GITHUB_REPOSITORY/releases?per_page=100"',
            "refusing to delete published release",
            'gh release delete "$RELEASE_TAG" --cleanup-tag --yes',
        ],
    )
    reject_fragments(
        release_please,
        RELEASE_PLEASE_WORKFLOW,
        [
            "workflow_dispatch",
            "RELEASE_PLEASE_TOKEN",
            "secrets.GITHUB_TOKEN",
            'releases/tags/$RELEASE_TAG',
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
            "ubuntu-24.04-lts",
            "signal-purple-release-ubuntu:",
            "signal-purple-${RELEASE_VERSION}-1.x86_64.rpm",
            "-F draft=false",
            "-F prerelease=false",
            "-f make_latest=true",
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
            "-F prerelease=true",
            "-f make_latest=false",
        ],
    )

    release_builder = (PROJECT_ROOT / RELEASE_BUILDER).read_text(
        encoding="utf-8"
    )
    require_fragments(
        release_builder,
        RELEASE_BUILDER,
        [
            "expected_os_id=debian",
            "expected_os_version=13",
            "expected_os_id=ubuntu",
            "expected_os_version=24.04",
            "/etc/os-release",
            '"$os_id" != "$expected_os_id"',
            '"$os_version" != "$expected_os_version"',
            "debug_package_extension=ddeb",
            'debug_package_name="signal-purple-dbgsym_${debian_version}_${architecture}.${debug_package_extension}"',
            'source_date_epoch=$(git -C "$repository" show -s --format=%ct "$commit")',
            "-ffile-prefix-map=$source_dir=/usr/src/signal-purple",
            "--remap-path-prefix=$source_dir=/usr/src/signal-purple",
            "export CFLAGS RUSTFLAGS",
            'export SOURCE_DATE_EPOCH="$source_date_epoch"',
        ],
    )

    cmake_config = (PROJECT_ROOT / CMAKE_CONFIG).read_text(encoding="utf-8")
    require_fragments(
        cmake_config,
        CMAKE_CONFIG,
        [
            "CPACK_RPM_SPEC_MORE_DEFINE",
            "%define _buildhost signal-purple-build.invalid",
            "%define use_source_date_epoch_as_buildtime 1",
            "%define build_mtime_policy clamp_to_source_date_epoch",
        ],
    )

    release_dockerfile = (PROJECT_ROOT / RELEASE_DOCKERFILE).read_text(
        encoding="utf-8"
    )
    require_fragments(
        release_dockerfile,
        RELEASE_DOCKERFILE,
        [
            "ARG UBUNTU_SNAPSHOT=20260805T000000Z",
            'test "$ID" = ubuntu',
            'test "$VERSION_ID" = 24.04',
        ],
    )

    apt_repository = (PROJECT_ROOT / APT_REPOSITORY_WORKFLOW).read_text(
        encoding="utf-8"
    )
    require_fragments(
        apt_repository,
        APT_REPOSITORY_WORKFLOW,
        [
            "workflow_call:",
            "workflow_dispatch:",
            "group: apt-repository-pages",
            'test "$GITHUB_REF" = "refs/heads/$DEFAULT_BRANCH"',
            "scripts/select-apt-release-assets.py",
            "scripts/build-apt-repository.sh",
            "debian-13",
            "ubuntu-24.04",
            "environment:\n      name: apt-repository",
            "APT_SIGNING_KEY_FINGERPRINT: "
            "${{ vars.APT_SIGNING_KEY_FINGERPRINT }}",
            "APT_SIGNING_PRIVATE_KEY: ${{ secrets.APT_SIGNING_PRIVATE_KEY }}",
            "APT_SIGNING_KEY_PASSPHRASE: "
            "${{ secrets.APT_SIGNING_KEY_PASSPHRASE }}",
            'test "$fingerprint" = "$APT_SIGNING_KEY_FINGERPRINT"',
            "signal-purple-archive-keyring.gpg",
            "signal-purple-archive-keyring.fingerprint",
            "--clearsign",
            "--detach-sign",
            "gpgv --keyring",
            "actions/upload-pages-artifact@"
            "fc324d3547104276b827a68afc52ff2a11cc49c9",
            "environment:\n      name: github-pages",
            "id-token: write",
            "pages: write",
            "actions/deploy-pages@"
            "cd2ce8fcbc39b97be8ca5fce6e763baed58fa128",
        ],
    )
    reject_fragments(
        apt_repository,
        APT_REPOSITORY_WORKFLOW,
        [
            "apt-key",
            "trusted=yes",
            "allow-unauthenticated",
            "types: [published]",
            "types: [released]",
            "keys/release-signing-key.asc",
        ],
    )

    try:
        prepare_job, remaining_jobs = apt_repository.split("\n  sign:\n", 1)
        sign_job, deploy_job = remaining_jobs.split("\n  deploy:\n", 1)
    except ValueError as error:
        fail(f"APT repository workflow has an invalid job boundary: {error}")
    reject_fragments(
        prepare_job,
        APT_REPOSITORY_WORKFLOW,
        [
            "APT_SIGNING_KEY_FINGERPRINT",
            "APT_SIGNING_PRIVATE_KEY",
            "APT_SIGNING_KEY_PASSPHRASE",
        ],
    )
    reject_fragments(
        sign_job,
        APT_REPOSITORY_WORKFLOW,
        ["actions/checkout@", "scripts/"],
    )
    require_fragments(
        deploy_job,
        APT_REPOSITORY_WORKFLOW,
        ["needs: sign", "pages: write", "id-token: write"],
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
