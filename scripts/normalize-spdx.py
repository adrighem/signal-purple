#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later

import datetime
import json
import os
import pathlib
import re
import sys
import tempfile


def fail(message: str) -> None:
    raise SystemExit(message)


if len(sys.argv) != 7:
    fail(
        "usage: normalize-spdx.py INPUT OUTPUT VERSION COMMIT EPOCH REPOSITORY"
    )

input_path = pathlib.Path(sys.argv[1])
output_path = pathlib.Path(sys.argv[2])
version, commit, epoch_text, repository = sys.argv[3:]

if not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", version):
    fail(f"invalid release version: {version}")
if not re.fullmatch(r"[0-9a-f]{40}", commit):
    fail(f"invalid release commit: {commit}")
if not re.fullmatch(r"[0-9]+", epoch_text):
    fail(f"invalid release epoch: {epoch_text}")
if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
    fail(f"invalid GitHub repository: {repository}")

with input_path.open(encoding="utf-8") as stream:
    document = json.load(stream)

if document.get("spdxVersion") != "SPDX-2.3":
    fail("SBOM is not SPDX 2.3")
if document.get("SPDXID") != "SPDXRef-DOCUMENT":
    fail("SBOM has an unexpected document identifier")
packages = document.get("packages")
if not isinstance(packages, list) or not packages:
    fail("SBOM contains no packages")
signal_core_packages = [
    package for package in packages if package.get("name") == "signal-core"
]
if len(signal_core_packages) != 1:
    fail("SBOM must contain exactly one signal-core package")
if signal_core_packages[0].get("versionInfo") != version:
    fail("SBOM signal-core version does not match the release")

created = datetime.datetime.fromtimestamp(
    int(epoch_text), tz=datetime.timezone.utc
).strftime("%Y-%m-%dT%H:%M:%SZ")
document["name"] = f"{repository}@v{version}"
document["documentNamespace"] = (
    f"https://github.com/{repository}/releases/tag/v{version}/spdx/{commit}"
)
document.setdefault("creationInfo", {})["created"] = created

output_path.parent.mkdir(parents=True, exist_ok=True)
with tempfile.NamedTemporaryFile(
    "w",
    encoding="utf-8",
    dir=output_path.parent,
    prefix=f".{output_path.name}.",
    delete=False,
) as stream:
    temporary_path = pathlib.Path(stream.name)
    json.dump(document, stream, indent=2, sort_keys=True)
    stream.write("\n")

os.replace(temporary_path, output_path)
