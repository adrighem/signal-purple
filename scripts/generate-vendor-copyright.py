#!/usr/bin/env python3
"""Generate deterministic DEP-5 file stanzas for a Cargo vendor directory."""

from __future__ import annotations

import argparse
import pathlib
import re
import tomllib


START = "# BEGIN GENERATED VENDOR INVENTORY"
END = "# END GENERATED VENDOR INVENTORY"
NOTICE_PREFIXES = ("COPYING", "LICENSE", "NOTICE")


def continuation(value: str) -> list[str]:
    lines = value.replace("\r", "").splitlines() or [value]
    return [" " + (line.strip() or ".") for line in lines]


def package_authors(package: dict[str, object]) -> list[str]:
    raw_authors = package.get("authors", [])
    if not isinstance(raw_authors, list):
        return []
    return sorted({str(author).strip() for author in raw_authors if str(author).strip()})


def license_expression(package: dict[str, object], package_dir: pathlib.Path) -> str:
    expression = package.get("license")
    if isinstance(expression, str) and expression.strip():
        return expression.strip().replace(" OR ", " or ").replace(" AND ", " and ")

    license_file = package.get("license-file")
    if isinstance(license_file, str) and license_file.strip():
        path = package_dir / license_file
        if not path.is_file():
            raise ValueError(f"missing declared license file: {path}")
        return f"LicenseRef-{package_dir.name}"

    raise ValueError(f"{package_dir.name} has no declared license")


def notice_files(package_dir: pathlib.Path) -> list[pathlib.Path]:
    return sorted(
        path
        for path in package_dir.iterdir()
        if path.is_file() and path.name.upper().startswith(NOTICE_PREFIXES)
    )


def copyright_lines(package: dict[str, object]) -> list[str]:
    authors = package_authors(package)
    if not authors:
        return ["the respective upstream authors and contributors"]
    return [authors[0], *authors[1:]]


def stanza(vendor_dir: pathlib.Path, manifest: pathlib.Path) -> str:
    data = tomllib.loads(manifest.read_text(encoding="utf-8"))
    package = data.get("package")
    if not isinstance(package, dict):
        raise ValueError(f"missing [package] table: {manifest}")

    relative_dir = manifest.parent.relative_to(vendor_dir.parent).as_posix()
    expression = license_expression(package, manifest.parent)
    authors = copyright_lines(package)
    notices = notice_files(manifest.parent)

    lines = [f"Files: {relative_dir}/*", f"Copyright: {authors[0]}"]
    lines.extend(f" {author}" for author in authors[1:])
    lines.append(f"License: {expression}")
    lines.extend(continuation("SPDX expression from the package's Cargo manifest."))
    if notices:
        paths = ", ".join(
            path.relative_to(vendor_dir.parent).as_posix() for path in notices
        )
        lines.extend(continuation(f"Bundled license and notice files: {paths}"))
    else:
        lines.extend(
            continuation(
                "The package contains no package-local license file; the declared "
                "standard license texts are present elsewhere in this complete "
                "vendored source graph."
            )
        )
    return "\n".join(lines)


def generate(vendor_dir: pathlib.Path) -> str:
    manifests = sorted(vendor_dir.glob("*/Cargo.toml"))
    if not manifests:
        raise ValueError(f"no vendored Cargo manifests found in {vendor_dir}")
    return "\n\n".join(stanza(vendor_dir, manifest) for manifest in manifests)


def replace_inventory(copyright_path: pathlib.Path, inventory: str) -> None:
    original = copyright_path.read_text(encoding="utf-8")
    pattern = re.compile(
        rf"^{re.escape(START)}$.*?^{re.escape(END)}$", re.MULTILINE | re.DOTALL
    )
    replacement = f"{START}\n{inventory}\n{END}"
    updated, count = pattern.subn(replacement, original)
    if count != 1:
        raise ValueError(f"expected one generated inventory block in {copyright_path}")
    copyright_path.write_text(updated, encoding="utf-8")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("vendor_dir", type=pathlib.Path)
    parser.add_argument("copyright_file", type=pathlib.Path)
    arguments = parser.parse_args()
    replace_inventory(arguments.copyright_file, generate(arguments.vendor_dir))


if __name__ == "__main__":
    main()
