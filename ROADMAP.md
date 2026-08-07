# Roadmap

signal-purple has a stable 1.x release line for its documented environment and
feature scope. Version numbers follow Semantic Versioning; compatibility work
advances when its evidence is recorded, not on a calendar deadline. Version
1.1.0 was the first published stable release.

## Stable maintenance priorities

- Preserve supported behavior, encrypted-store migrations, and the paired
  plugin/backend lifecycle across updates.
- Respond promptly to Signal service drift, security issues, and regressions.
- Keep release inputs pinned and publish reproducible, attested packages for
  Debian 13 and Ubuntu 24.04 LTS.
- Keep resource limits, data retention, unsupported features, and recovery
  procedures explicit.
- Add cursor pagination to the Presage unprojected-message API so peak startup
  memory no longer scales with the complete offline backlog.

## Compatibility evidence

- Expand revision-specific live validation for group messages, remote leave,
  attachments, typing and receipts, identity replacement, offline delivery, and
  network recovery.
- Re-run affected scenarios before releasing protocol, storage, or Signal-stack
  changes.
- Keep automated coverage and production-service evidence distinct.

## Scope evolution

- Add capabilities only with bounded resource use, safe migrations, documented
  recovery, and focused tests.
- Keep primary registration, calls, backups, and Purple 3 outside the supported
  scope until an explicit design and maintenance commitment exists.

Completed release history is in the [changelog](CHANGELOG.md). Requirements for
each new stable release are in the
[stable release checklist](docs/release-checklist.md).
