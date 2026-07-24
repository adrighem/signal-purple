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

document_describes = document.get("documentDescribes", [])
if not isinstance(document_describes, list) or not all(
    isinstance(identifier, str) for identifier in document_describes
):
    fail("SBOM has invalid described-package identifiers")
described_ids = set(document_describes)
relationships = document.get("relationships", [])
if not isinstance(relationships, list):
    fail("SBOM has invalid relationships")
for relationship in relationships:
    if not isinstance(relationship, dict):
        fail("SBOM has invalid relationship")
    if (
        relationship.get("spdxElementId") == "SPDXRef-DOCUMENT"
        and relationship.get("relationshipType") == "DESCRIBES"
    ):
        related = relationship.get("relatedSpdxElement")
        if not isinstance(related, str):
            fail("SBOM has invalid described-package relationship")
        described_ids.add(related)

described_roots = [
    package
    for package in packages
    if package.get("SPDXID") in described_ids
    and isinstance(package.get("name"), str)
    and pathlib.PurePosixPath(package["name"]).is_absolute()
]
if len(described_roots) != 1:
    fail("SBOM must have exactly one absolute described root package")

root = described_roots[0]
old_root_id = root.get("SPDXID")
if not isinstance(old_root_id, str):
    fail("SBOM root package has no valid SPDX identifier")
new_root_id = "SPDXRef-DocumentRoot-Directory-signal-purple"
if any(
    package is not root and package.get("SPDXID") == new_root_id
    for package in packages
):
    fail("SBOM already contains the normalized root SPDX identifier")

root["name"] = f"{repository}@v{version}"
root["SPDXID"] = new_root_id
document["documentDescribes"] = [
    new_root_id if identifier == old_root_id else identifier
    for identifier in document_describes
]
for relationship in relationships:
    for field in ("spdxElementId", "relatedSpdxElement"):
        if relationship.get(field) == old_root_id:
            relationship[field] = new_root_id

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
