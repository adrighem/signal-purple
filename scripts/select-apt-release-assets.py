#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later

import json
import pathlib
import re
import sys


TAG_PATTERN = re.compile(r"^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")
DIGEST_PATTERN = re.compile(r"^sha256:[0-9a-f]{64}$")
DISTRO_IDS = ("debian-13", "ubuntu-24.04-lts")


def fail(message: str) -> None:
    raise SystemExit(message)


def distro_names(tag: str) -> tuple[str, ...]:
    version = tag[1:]
    return tuple(
        name
        for distro_id in DISTRO_IDS
        for name in (
            f"signal-purple_{version}-1_{distro_id}_amd64.deb",
            f"signal-purple-dbgsym_{version}-1_{distro_id}_amd64.deb",
        )
    )


def legacy_debian_names(tag: str) -> tuple[str, str]:
    version = tag[1:]
    return (
        f"signal-purple_{version}-1_amd64.deb",
        f"signal-purple-dbgsym_{version}-1_amd64.deb",
    )


def required_names(
    tag: str, by_name: dict[str, dict[str, object]], newest: bool
) -> tuple[str, ...]:
    current = distro_names(tag)
    present = tuple(name in by_name for name in current)
    if newest or any(present):
        if not all(present):
            missing = next(name for name in current if name not in by_name)
            fail(f"stable release {tag} is missing required asset {missing}")
        return current
    return legacy_debian_names(tag)


def version_key(release: dict[str, object]) -> tuple[int, int, int]:
    match = TAG_PATTERN.fullmatch(str(release["tag_name"]))
    if match is None:
        fail("stable release has an invalid semantic-version tag")
    return tuple(int(part) for part in match.groups())


def select_assets(releases: object) -> list[tuple[str, str, int, str]]:
    if not isinstance(releases, list):
        fail("GitHub releases response must be an array")

    stable: list[dict[str, object]] = []
    for release in releases:
        if not isinstance(release, dict):
            fail("GitHub releases response contains a non-object")
        if release.get("draft") is False and release.get("prerelease") is False:
            tag = release.get("tag_name")
            published_at = release.get("published_at")
            if not isinstance(tag, str) or TAG_PATTERN.fullmatch(tag) is None:
                fail("stable release has an invalid semantic-version tag")
            if not isinstance(published_at, str) or not published_at:
                fail(f"stable release {tag} has no publication timestamp")
            stable.append(release)

    stable.sort(key=version_key, reverse=True)
    selected = stable[:2]
    if not selected:
        fail("no published stable release is available for the APT repository")

    result: list[tuple[str, str, int, str]] = []
    for release_index, release in enumerate(selected):
        tag = str(release["tag_name"])
        assets = release.get("assets")
        if not isinstance(assets, list):
            fail(f"stable release {tag} has no asset array")

        by_name: dict[str, dict[str, object]] = {}
        for asset in assets:
            if not isinstance(asset, dict):
                fail(f"stable release {tag} contains a non-object asset")
            name = asset.get("name")
            if not isinstance(name, str):
                fail(f"stable release {tag} contains an unnamed asset")
            if name in by_name:
                fail(f"stable release {tag} contains duplicate asset {name}")
            by_name[name] = asset

        for name in required_names(tag, by_name, release_index == 0):
            asset = by_name.get(name)
            if asset is None:
                fail(f"stable release {tag} is missing required asset {name}")
            asset_id = asset.get("id")
            digest = asset.get("digest")
            size = asset.get("size")
            state = asset.get("state")
            if not isinstance(asset_id, int) or asset_id <= 0:
                fail(f"release asset {name} has an invalid ID")
            if not isinstance(digest, str) or DIGEST_PATTERN.fullmatch(digest) is None:
                fail(f"release asset {name} has no valid SHA-256 digest")
            if not isinstance(size, int) or size <= 0 or state != "uploaded":
                fail(f"release asset {name} is not a complete upload")
            result.append((tag, name, asset_id, digest))

    return result


def main() -> None:
    if len(sys.argv) != 2:
        fail(f"usage: {pathlib.Path(sys.argv[0]).name} RELEASES_JSON")
    path = pathlib.Path(sys.argv[1])
    try:
        releases = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(f"cannot read GitHub releases response: {error}")

    for tag, name, asset_id, digest in select_assets(releases):
        print(tag, name, asset_id, digest, sep="\t")


if __name__ == "__main__":
    main()
