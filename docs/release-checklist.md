# Stable release checklist

This document defines requirements for every stable release. One candidate
issue records pass, fail, or not-applicable status and links evidence for each
requirement. This file states the contract; it does not claim that a particular
candidate passed. Historical evidence and known gaps remain in
[live-validation.md](live-validation.md).

The candidate is the reviewed release pull-request tree. Merging that pull
request is release approval: Release Please tags the resulting `main` commit,
creates a draft, and publishes it only after the automated artifact gates pass.
Stable status covers the supported project scope. It does not turn an untested
Signal service scenario into a compatibility claim.

## Supported scope

- Debian 13 and Ubuntu 24.04 LTS with libpurple 2.
- One Signal account per configured Purple account.
- Direct messages, contact synchronization, typing, delivery, read, retry, and
  messages received while this client was offline.
- Group discovery, title and membership synchronization, and group messaging.
- Identity-change warning and acceptance without relinking.
- Attachments within documented size and resource limits.
- Upgrade without losing the account, contacts, or trust state.

## Out of scope

- Signal registration, account recovery, or primary-device replacement.
- Calls, stories, payments, and unsupported Signal experiments.
- Operating systems or libpurple versions not named above.

## Build and supply chain

- Release inputs and executable workflow actions are pinned or otherwise
  covered by an explicit trust decision.
- Primary CI passes formatting, warnings, tests, cross-language ABI checks, and
  the staged module-load probe.
- Clean Debian 13 and Ubuntu 24.04 LTS jobs build, test, install, and probe the
  candidate.
- The vendored source archive produces the supported distro packages and the
  best-effort Fedora RPM twice with identical contents.
- Source archive, packages, checksums, SBOM, and provenance attestations agree.
- No known unresolved release-blocking vulnerability remains.
- Every product-version consumer agrees with `version.txt` or derives from it.

## Interoperability

- Review changes since the previous stable release against the complete matrix
  in [compatibility.md](compatibility.md).
- Run affected live scenarios for protocol, storage, and Signal-stack changes
  with dedicated non-production accounts.
- Record the tested Signal clients, date, candidate revision, results, and every
  untested scenario without sensitive account or message data.
- Keep deterministic coverage for direct messages, offline replay, contact and
  group synchronization, typing and receipts, identity replacement, and bounded
  attachments.
- Do not claim compatibility for a service/client scenario without
  revision-specific evidence.

## Resilience and safety

- Exercise recovery paths affected by the candidate, including network loss,
  reconnects, storage faults, queue pressure, and remote protocol errors.
- Confirm corrupt state, unavailable key storage, and full disk fail safely.
- Confirm idle operation has no recurring backend poll wakeups or hot
  Pidgin/`signal-purple-core` thread when scheduling changes.
- Review diagnostics for credentials, message content, and private identifiers.
- Keep focused malformed-input coverage at the C/Rust ABI.
- Run AddressSanitizer and UndefinedBehaviorSanitizer for affected C ownership
  and lifecycle changes.
- Exercise upgrade and rollback when storage, packaging, or installation paths
  change.
- Complete a proportionate soak with no unresolved release-blocking regression.

## Documentation and release

- User installation, upgrade, rollback, relink, removal, compatibility,
  security, data-retention, and support documentation matches the candidate.
- Candidate install, load, upgrade, rollback, and uninstall paths pass for each
  advertised installation scope affected by the release.
- The release pull request matches the audited changelog and product version.
- The Release Please tag identifies the merged release commit.
- Artifacts are reproduced, attested, smoke-tested, and attached before the
  draft is published as stable.
- Downloaded release assets match `SHA256SUMS`.
- Stable publication deploys signed APT metadata, and isolated Debian 13 and
  Ubuntu 24.04 clients select their matching packages while the repository
  retains the supported predecessor.
- A rollback decision and recovery path exist before publication.
