# Decisions

## 2026-07-18 — initial architecture

- Target Purple 2.14 on Debian 13.
- Use an owned polling C ABI and a Presage-backed Rust actor.
- Link as a secondary device; defer primary registration.
- Encrypt Presage storage with SQLCipher and hold its passphrase in libsecret.
- License original C/project work GPL-3.0-or-later and the Rust backend
  AGPL-3.0-only; document combined-distribution obligations.

## 2026-07-19 - contact buddy-list ownership

- Request a fresh contact sync from the primary device on every connection;
  an unsolicited initial queue is not guaranteed to contain the full address
  book.
- Treat a successfully decoded Signal contact snapshot as authoritative for
  plugin-managed Purple buddies.
- Preserve user-created, unmanaged buddies and local aliases.
- Apply deletions only after an explicit completed snapshot boundary; a store
  read or decode error must leave the existing buddy list intact.
- Treat synchronized contacts as reachable while connected because Signal has
  no presence API and Purple otherwise hides the address book by default.
- Keep Signal contact mutation and phone-number discovery out of scope.

## 2026-07-21 — release candidate identity

- Land all runtime, storage, dependency, packaging, CI, and release-hardening
  inputs before freezing a candidate.
- Merge the version/changelog pull request to establish the candidate on
  `main`; that merge does not tag or publish the release.
- Record and validate the resulting post-merge `main` SHA. A pull-request head
  is not a substitute when GitHub creates a distinct merge commit or artifact
  metadata depends on commit timestamps.
- Keep the validation tracker open through packaging, signing, and publication.
- Treat 0.1.0 as an unpublished versioning bootstrap and use cumulative notes
  for the first intended public pre-release, 0.2.0.

## 2026-07-21 — 0.2.0 live-validation waiver

- Proceed with 0.2.0 only as an explicitly labelled pre-alpha without the
  dedicated-account live interoperability, network-recovery, idle/diagnostic,
  or soak exercises.
- Keep every waived gate unchecked and describe it as unverified rather than
  passed. Do not claim production-service compatibility or an official-client
  version.
- Keep artifact and tag signing as a release blocker until the release owner
  separately approves a suitable signing-key path.

## 2026-07-21 — 0.2.0 publication

- Publish 0.2.0 only from signed tag `v0.2.0` at reviewed commit
  `59d4f257a3b2514261d3fc773da4e9df90d9ffd4`.
- Use signing fingerprint
  `B3C0B75FA3B33AC278738C5CB1798BCDA76054BD` for both the tag and checksum
  manifest; retain the GitHub noreply UID so signature verification and email
  privacy are compatible.
- Preserve the live-validation waiver as a prominent pre-alpha limitation;
  publication does not convert any waived check into a pass.

## 2026-07-25 — Release Please owns releases

- For future releases, merging the reviewed Release Please pull request is the
  release approval. This supersedes the manual-tag portion of the 2026-07-21
  candidate-identity decision without changing historical releases.
- Release Please owns the version change, changelog, canonical tag, and draft
  GitHub release. Its exact tag, version, and commit outputs feed the artifact
  workflow directly, avoiding event behavior that differs between a personal
  token and `GITHUB_TOKEN`.
- Keep releases private until reproducible builds, package probing, SBOM and
  checksum generation, provenance attestation, and asset upload all pass. The
  final trusted job publishes the existing draft as a prerelease that is not
  marked `Latest`; it never creates or moves a release tag.
- Replace maintainer-signed tag trust with protected `main`, a
  checksum-pinned Release Please action, exact tag/commit/version checks,
  least-privilege jobs, and OIDC artifact provenance. Retain the existing
  OpenPGP public key only for historical verification.
- Authenticate Release Please with a private, repository-scoped GitHub App
  limited to Contents and Pull requests read/write. Do not fall back to
  `GITHUB_TOKEN`: missing App configuration must fail visibly, while
  App-authored release pull requests start their checks automatically.

## 2026-07-25 — Refactor along owned boundaries

- Prefer small behavior-preserving extractions over splitting the backend actor
  or Purple connection lifecycle in one change.
- Event delivery owns its producer, consumer, descriptor notification,
  overflow state, and byte accounting in one Rust module.
- Cargo remains the Rust dependency authority; CMake invokes its incremental
  build rather than maintaining a second source-file list.
- `scripts/check.sh` is the canonical contributor and primary-CI validation
  entry point. The clean Debian 13 job remains the supported-platform gate.
- Track the remaining attachment-admission, recovery-state, connection
  lifecycle, acknowledgment, and ABI-conformance work in
  `work/architecture-refactor-2026-07-25.md`.

## 2026-07-31 — Keep attached video out of the image path

- Render genuine GIF payloads through Purple's existing image store only after
  MIME, signature, full-decode, compressed-size, dimension, and cumulative
  frame-area validation.
- Keep MP4 payloads, including Signal GIF-style animations transported as MP4,
  in Purple's bounded receive-file flow. Do not pass video bytes to the image
  store, invoke an automatic external transcoder, or create a plugin-managed
  plaintext media cache.
- Give unnamed common media deterministic type-specific filenames so the
  fallback transfer remains usable. Preserve any sender-provided filename.

## 2026-07-31 - Permit bounded Signal GIF presentation conversion

- Supersede the earlier prohibition on automatic transcoding only for incoming
  group MP4 attachments which carry Signal's GIF flag and a valid MP4 file-type
  box. Ordinary video remains in Purple's receive-file flow.
- Keep conversion optional and outside the process through Debian's FFmpeg and
  `prlimit`, fixed arguments, a cleared environment, memory-only pipes, and
  explicit input, output, CPU, memory, descriptor, thread, wall-time,
  concurrency, attempt, frame-rate, dimension, and frame-area limits.
- Replace the attachment presentation only after the generated GIF passes the
  native structure and aggregate message budgets. Preserve the original MP4
  for every unavailable, failed, contended, invalid, or over-budget conversion.
- Recommend the runtime helpers rather than linking libav, keeping dependency
  ABI churn and media-decoder failures outside the plugin process.
