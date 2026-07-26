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
- Text, attachment, typing, delivery-receipt, and read-receipt timestamps
  allocated by signal-purple must share one per-core sequence. Every new
  allocation must be unique and greater than the preceding allocation,
  including under concurrent attachment work and wall-clock rollback. Durable
  retries reuse the logical message's original timestamp. Presage-owned
  contact-sync requests and group-leave peer notifications are outside this
  sequence.
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
  `QUEUE_FULL`, and accepted acknowledgements are eventually persisted.
- An accepted cancellation cannot later start or continue its upload task,
  including when it overtakes a queued send.
- Closing a connection severs every live pending transfer from its plugin
  context before freeing connection state, so late callbacks are safe.
- A sparse file larger than 25 MiB is rejected without reading its contents.
- Repeated and concurrent allocation through the signal-purple timestamp
  allocator, including simulated wall-clock rollback, produces unique, strictly
  increasing values. Durable retries preserve their original timestamp.
- Synchronization, replay, outbox, and attachment-download waits selected
  through the shutdown boundary terminate promptly.
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
- Phase-completion verification is offline and uses deterministic test
  fixtures. Live Signal service validation requires separate authorization and
  remains out of scope.
