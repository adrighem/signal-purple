# Track Specification

## Overview

Resolve the highest-risk architecture-review findings without widening the
public feature set or changing the Signal Core ABI version.

## Background

The current owned ABI and bounded attachment/event work are sound foundations,
but several correctness-critical actions still share a best-effort work queue.
Frontend transfer callbacks can also outlive connection state, local file
limits are checked after full allocation, outgoing timestamps can collide, and
teardown can wait on uncancelled network work.

## Functional Requirements

- Message display acknowledgements must remain accepted when the ordinary work
  queue is full.
- Cancelling an admitted attachment must record cancellation without depending
  on command-channel capacity.
- Every created outgoing transfer must be detached before its connection is
  freed.
- Oversized or non-regular outgoing files must be rejected before full
  allocation.
- Locally generated Signal message timestamps must be strictly increasing
  within a core, including concurrent attachment tasks.
- Worker teardown must be able to interrupt synchronization, replay, outbox,
  and attachment-download waits.
- The SQLCipher passphrase must enter zeroizing ownership immediately and be
  dropped after store opening.

## Non-Functional Requirements

- Preserve ABI version 7 and existing event/status values.
- Keep Purple calls on the main thread and Presage work on the backend
  `LocalSet`.
- Keep attachment count and byte admission limits unchanged.
- Add deterministic pressure and lifecycle regression tests.
- Run validation under a sanitized environment.

## Acceptance Criteria

- More acknowledgements than the work-channel capacity can be queued without
  `QUEUE_FULL`.
- An accepted cancellation cannot later start or continue its upload task.
- Closing a connection nulls every pending outgoing transfer's connection
  pointer.
- A sparse file larger than 25 MiB is rejected without reading its contents.
- Repeated and concurrent timestamp allocation produces unique increasing
  values.
- A pending future selected through the shutdown boundary terminates promptly.
- Constructor and worker code never hold the passphrase in a plain `String`
  after validation and do not retain it for the session lifetime.
- Focused suites and `scripts/check.sh full` pass.

## Out of Scope

- Changing the upstream Presage projection schema or attachment downloader.
- Live production-service validation.
- New Signal or Purple features.
- ABI version changes.

## Dependencies

- Existing libpurple, GLib, Tokio, and pinned Presage APIs.
- Manual end-to-end verification requires an isolated non-production Signal
  profile and remains a phase-completion step.
