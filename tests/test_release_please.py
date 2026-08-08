#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later

import json
import pathlib
import re
import tomllib
import xml.etree.ElementTree as ET


PROJECT_ROOT = pathlib.Path(__file__).resolve().parents[1]
CI_WORKFLOW = pathlib.PurePosixPath(".github/workflows/ci.yml")
WORKFLOWS_DIRECTORY = pathlib.PurePosixPath(".github/workflows")
CARGO_MANIFEST = pathlib.PurePosixPath("rust/signal-core/Cargo.toml")
LOCK_PATH = pathlib.PurePosixPath("rust/signal-core/Cargo.lock")
DEPENDENCY_POLICY = pathlib.PurePosixPath("docs/dependency-policy.md")
THIRD_PARTY_LICENSES = pathlib.PurePosixPath("THIRD_PARTY_LICENSES.md")
MARKER = "# x-release-please-version"
RELEASE_ARTIFACTS_WORKFLOW = pathlib.PurePosixPath(
    ".github/workflows/release-artifacts.yml"
)
APT_REPOSITORY_WORKFLOW = pathlib.PurePosixPath(
    ".github/workflows/apt-repository.yml"
)
APPSTREAM_METADATA = pathlib.PurePosixPath(
    "data/io.github.adrighem.signal-purple.metainfo.xml"
)
RELEASE_PLEASE_WORKFLOW = pathlib.PurePosixPath(
    ".github/workflows/release-please.yml"
)
RELEASE_BUILDER = pathlib.PurePosixPath("scripts/build-release-artifacts.sh")
RELEASE_DOCKERFILE = pathlib.PurePosixPath(".github/release/Dockerfile")
CMAKE_CONFIG = pathlib.PurePosixPath("CMakeLists.txt")
NIX_FLAKE = pathlib.PurePosixPath("flake.nix")
EXPECTED_CACHE_ACTION_SHA = "0057852bfaa89a56745cba8c7296529d2fc39830"


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


def validate_ci_workflow() -> None:
    ci_text = (PROJECT_ROOT / CI_WORKFLOW).read_text(encoding="utf-8")
    for job_name in (
        "build-and-test",
        "debian-13-build-and-install",
        "ubuntu-24-04-build-and-install",
    ):
        match = re.search(
            rf"(?ms)^  {re.escape(job_name)}:\n(.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)",
            ci_text,
        )
        if match is None:
            fail(f"{CI_WORKFLOW} is missing required job: {job_name}")
        require_fragments(
            match.group(1),
            CI_WORKFLOW,
            [
                'SIGNAL_PURPLE_REQUIRE_FFMPEG_TEST: "1"',
                "ffmpeg",
                "util-linux",
            ],
        )

    for job_name in (
        "debian-13-build-and-install",
        "ubuntu-24-04-build-and-install",
    ):
        match = re.search(
            rf"(?ms)^  {re.escape(job_name)}:\n(.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)",
            ci_text,
        )
        require_fragments(match.group(1), CI_WORKFLOW, ["scripts/check.sh rust-test"])

    cache_uses = []
    workflow_root = PROJECT_ROOT / WORKFLOWS_DIRECTORY
    workflow_paths = sorted(workflow_root.glob("*.yml"))
    workflow_paths.extend(sorted(workflow_root.glob("*.yaml")))
    for workflow_path in workflow_paths:
        relative_path = pathlib.PurePosixPath(
            workflow_path.relative_to(PROJECT_ROOT).as_posix()
        )
        for line_number, line in enumerate(
            workflow_path.read_text(encoding="utf-8").splitlines(), start=1
        ):
            if "actions/cache@" not in line:
                continue
            match = re.fullmatch(
                r"\s*uses:\s*actions/cache@([^\s#]+)\s*(?:#\s*(.*))?", line
            )
            if match is None:
                fail(
                    f"{relative_path}:{line_number} has an invalid actions/cache "
                    "declaration"
                )
            reference = match.group(1)
            if re.fullmatch(r"[0-9a-f]{40}", reference) is None:
                fail(
                    f"{relative_path}:{line_number} uses mutable actions/cache "
                    f"reference: {reference}"
                )
            cache_uses.append(
                (relative_path, reference, (match.group(2) or "").strip())
            )

    expected_cache_uses = [
        (CI_WORKFLOW, EXPECTED_CACHE_ACTION_SHA, "v4.3.0"),
        (CI_WORKFLOW, EXPECTED_CACHE_ACTION_SHA, "v4.3.0"),
        (CI_WORKFLOW, EXPECTED_CACHE_ACTION_SHA, "v4.3.0"),
    ]
    if cache_uses != expected_cache_uses:
        fail(
            "GitHub workflows must contain exactly three actions/cache v4.3.0 "
            f"uses pinned to {EXPECTED_CACHE_ACTION_SHA}"
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


def validate_presage_revision() -> None:
    manifest = tomllib.loads(
        (PROJECT_ROOT / CARGO_MANIFEST).read_text(encoding="utf-8")
    )
    presage_dependencies = [
        manifest["dependencies"]["presage"],
        manifest["dependencies"]["presage-store-sqlite"],
    ]
    revisions = {dependency.get("rev") for dependency in presage_dependencies}
    repositories = {dependency.get("git") for dependency in presage_dependencies}
    if len(revisions) != 1 or None in revisions:
        fail("Presage dependencies must use one exact Git revision")
    if repositories != {"https://github.com/adrighem/presage.git"}:
        fail("Presage dependencies must use the documented public fork")
    revision = revisions.pop()
    if re.fullmatch(r"[0-9a-f]{40}", revision) is None:
        fail("Presage dependencies must use a full 40-character Git revision")

    lock = tomllib.loads((PROJECT_ROOT / LOCK_PATH).read_text(encoding="utf-8"))
    expected_source = (
        "git+https://github.com/adrighem/presage.git"
        f"?rev={revision}#{revision}"
    )
    for package_name in ("presage", "presage-store-sqlite"):
        packages = [
            package
            for package in lock["package"]
            if package.get("name") == package_name
        ]
        if len(packages) != 1 or packages[0].get("source") != expected_source:
            fail(f"Cargo.lock does not pin {package_name} to {revision}")

    policy_text = (PROJECT_ROOT / DEPENDENCY_POLICY).read_text(encoding="utf-8")
    policy_revision = re.search(
        r"The Presage dependency.*?revision `([0-9a-f]{40})`", policy_text, re.DOTALL
    )
    if policy_revision is None or policy_revision.group(1) != revision:
        fail("dependency policy Presage revision does not match Cargo.toml")
    licenses_text = (PROJECT_ROOT / THIRD_PARTY_LICENSES).read_text(encoding="utf-8")
    if f"| `{revision}` (fork base:" not in licenses_text:
        fail("third-party license Presage revision does not match Cargo.toml")


def validate_nix_version_source() -> None:
    flake_text = (PROJECT_ROOT / NIX_FLAKE).read_text(encoding="utf-8")
    require_fragments(
        flake_text,
        NIX_FLAKE,
        ["pkgs.lib.removeSuffix", "builtins.readFile ./version.txt"],
    )
    if re.search(r'(?m)^\s*version\s*=\s*"[0-9]+\.[0-9]+\.[0-9]+";', flake_text):
        fail("flake.nix must derive the package version from version.txt")
    if re.search(
        r"(?m)^\s*license\s*=\s*\[\s*licenses\.gpl3Plus\s+"
        r"licenses\.agpl3Only\s*\];\s*$",
        flake_text,
    ) is None:
        fail("flake.nix must declare both the GPL adapter and AGPL backend")


def validate_appstream_metadata() -> None:
    root = ET.parse(PROJECT_ROOT / APPSTREAM_METADATA).getroot()
    description = " ".join(root.find("description").itertext()).split()
    description_text = " ".join(description)
    require_fragments(
        description_text,
        APPSTREAM_METADATA,
        [
            "independent linked-device Signal protocol plugin",
            "Published 1.x releases are stable",
            "Debian 13 and Ubuntu 24.04 LTS scope",
            "not supported by Signal",
        ],
    )
    developer = root.find("developer")
    if (
        developer is None
        or developer.get("id") != "io.github.adrighem"
        or developer.findtext("name") != "signal-purple contributors"
    ):
        fail(f"{APPSTREAM_METADATA} must identify the project developer")
    if root.find("developer_name") is not None:
        fail(f"{APPSTREAM_METADATA} must not use deprecated developer_name")


def main() -> None:
    validate_release_config()
    validate_release_workflows()
    validate_ci_workflow()
    validate_lock_marker()
    validate_presage_revision()
    validate_nix_version_source()
    validate_appstream_metadata()


if __name__ == "__main__":
    main()
