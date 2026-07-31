# Architecture refactor — 2026-07-25

## Goal

Improve code navigation, contributor confidence, and build correctness without
combining behavioral changes with a large backend or connection rewrite.

## Implemented slice

1. Make event delivery one owned Rust subsystem, including its descriptor
   notification, overflow state, and byte accounting.
2. Test event-queue boundaries with an injected small byte budget instead of
   parallel 64 MiB allocations.
3. Convert attachment-task panics into request-scoped failures so transfers do
   not remain pending indefinitely.
4. Let Cargo decide whether Rust sources need rebuilding whenever CMake builds,
   removing the manually maintained module dependency list.
5. Provide one check driver used by local development and the primary CI job,
   including a reusable staged-install probe.
6. Document subsystem ownership, Conventional Commit titles, Release Please
   file ownership, the complete local gate, and the current alpha security
   policy.
## Follow-up architecture backlog

These deserve separate behavior-focused changes and are intentionally excluded
from the extraction above:

- ~~Bound aggregate queued and in-flight attachment bytes and upload
  concurrency; reject duplicate attachment request identifiers.~~ Completed in
  [the attachment-admission phase](attachment-admission-2026-07-25.md).
- ~~Make attachment cancellation reliable even when the command channel is
  full, and keep C transfer ownership until cancellation is accepted.~~
  Completed in the v0.3.0 reliability-hardening phase.
- ~~Read outgoing local files through a bounded single-open C helper so
  oversized files are rejected before a full allocation.~~ Completed in the
  v0.3.0 reliability-hardening phase.
- Extract a deterministic recovery/session transition model from the backend
  actor before splitting contact, group, projection, and outbox modules.
- Centralize `SignalConnection` construction and teardown while preserving
  Purple callback-detachment order.
- ~~Prevent malformed direct, group, or attachment events from being
  projection-acknowledged unless the adapter accepted them.~~ Completed by
  separating projection acceptance from the event source's keep-running result.
- Add compiled C/Rust ABI constant, layout, and public-limit conformance tests.
- Split installation lifecycle guidance from maintainer release automation and
  add a pinned C formatting gate.

Each follow-up should land independently with focused regression coverage,
full `scripts/check.sh full` validation, and the Debian 13 CI gate.
