#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu

project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
temporary=$(mktemp -d "${TMPDIR:-/tmp}/signal-purple-release-test.XXXXXX")
trap 'rm -rf "$temporary"' EXIT HUP INT TERM

expected_fingerprint=B3C0B75FA3B33AC278738C5CB1798BCDA76054BD
key_listing=$(gpg --batch --with-colons --show-keys \
    "$project_root/keys/release-signing-key.asc")
test "$(printf '%s\n' "$key_listing" \
    | awk -F: '$1 == "pub" { count++ } END { print count + 0 }')" -eq 1
actual_fingerprint=$(printf '%s\n' "$key_listing" \
    | awk -F: '$1 == "fpr" { print $10; exit }')
test "$actual_fingerprint" = "$expected_fingerprint"

profile_output="$temporary/profile-output"
profile_status=0
dpkg-checkbuilddeps -Ppkg.signal-purple.upstream-rust \
    "$project_root/debian/control" >"$profile_output" 2>&1 \
    || profile_status=$?
case "$profile_status" in
    0 | 1) ;;
    *)
        cat "$profile_output" >&2
        exit "$profile_status"
        ;;
esac
if grep -Eq '(^|[ ,])(cargo|rustc)([ (]|$)' "$profile_output"; then
    printf '%s\n' 'upstream Rust profile did not suppress Rust packages' >&2
    cat "$profile_output" >&2
    exit 1
fi

artifacts="$temporary/artifacts"
mkdir "$artifacts"
printf '%s\n' source > "$artifacts/signal-purple_1.2.3.orig.tar.xz"
printf '%s\n' runtime > "$artifacts/signal-purple_1.2.3-1_amd64.deb"
printf '%s\n' debug > "$artifacts/signal-purple-dbgsym_1.2.3-1_amd64.deb"
printf '%s\n' sbom > "$artifacts/signal-purple-1.2.3.spdx.json"
"$project_root/scripts/write-release-checksums.sh" 1.2.3 "$artifacts"
test "$(wc -l < "$artifacts/SHA256SUMS")" -eq 4
(cd "$artifacts" && sha256sum --check SHA256SUMS)

if "$project_root/scripts/write-release-checksums.sh" 1.2 "$artifacts" \
    > /dev/null 2>&1; then
    printf '%s\n' 'invalid version unexpectedly produced checksums' >&2
    exit 1
fi

missing="$temporary/missing"
mkdir "$missing"
printf '%s\n' source > "$missing/signal-purple_1.2.3.orig.tar.xz"
if "$project_root/scripts/write-release-checksums.sh" 1.2.3 "$missing" \
    > /dev/null 2>&1; then
    printf '%s\n' 'incomplete artifact set unexpectedly passed' >&2
    exit 1
fi

raw_sbom="$temporary/raw.spdx.json"
normalized_a="$temporary/normalized-a.spdx.json"
normalized_b="$temporary/normalized-b.spdx.json"
cat > "$raw_sbom" <<'EOF'
{
  "SPDXID": "SPDXRef-DOCUMENT",
  "spdxVersion": "SPDX-2.3",
  "name": "variable",
  "documentNamespace": "https://example.invalid/random",
  "creationInfo": {
    "created": "2026-01-01T00:00:00Z",
    "creators": ["Tool: test"]
  },
  "packages": [
    {
      "SPDXID": "SPDXRef-DocumentRoot-Directory--home-runner-work-temp",
      "name": "/home/runner/work/_temp/release-sbom-source/signal-purple-1.2.3"
    },
    {
      "SPDXID": "SPDXRef-Package-signal-core",
      "name": "signal-core",
      "versionInfo": "1.2.3"
    }
  ],
  "relationships": [
    {
      "spdxElementId": "SPDXRef-DOCUMENT",
      "relatedSpdxElement": "SPDXRef-DocumentRoot-Directory--home-runner-work-temp",
      "relationshipType": "DESCRIBES"
    },
    {
      "spdxElementId": "SPDXRef-DocumentRoot-Directory--home-runner-work-temp",
      "relatedSpdxElement": "SPDXRef-Package-signal-core",
      "relationshipType": "CONTAINS"
    }
  ]
}
EOF
"$project_root/scripts/normalize-spdx.py" \
    "$raw_sbom" "$normalized_a" 1.2.3 \
    0123456789abcdef0123456789abcdef01234567 1767225600 \
    adrighem/signal-purple
"$project_root/scripts/normalize-spdx.py" \
    "$raw_sbom" "$normalized_b" 1.2.3 \
    0123456789abcdef0123456789abcdef01234567 1767225600 \
    adrighem/signal-purple
cmp "$normalized_a" "$normalized_b"
python3 - "$normalized_a" <<'PY'
import json
import pathlib
import sys

document = json.loads(pathlib.Path(sys.argv[1]).read_text())
assert document["name"] == "adrighem/signal-purple@v1.2.3"
assert document["creationInfo"]["created"] == "2026-01-01T00:00:00Z"
assert document["documentNamespace"].endswith(
    "/spdx/0123456789abcdef0123456789abcdef01234567"
)
root = next(
    package for package in document["packages"]
    if package["SPDXID"] == "SPDXRef-DocumentRoot-Directory-signal-purple"
)
assert root["name"] == "adrighem/signal-purple@v1.2.3"
assert document["relationships"][0]["relatedSpdxElement"] == root["SPDXID"]
assert document["relationships"][1]["spdxElementId"] == root["SPDXID"]
assert "/home/runner/" not in json.dumps(document)
PY

if "$project_root/scripts/normalize-spdx.py" \
    "$raw_sbom" "$temporary/wrong-version.spdx.json" 1.2.4 \
    0123456789abcdef0123456789abcdef01234567 1767225600 \
    adrighem/signal-purple > /dev/null 2>&1; then
    printf '%s\n' 'mismatched SBOM package version unexpectedly passed' >&2
    exit 1
fi
