# Outgoing attachment admission — 2026-07-25

## Goal

Prevent the bounded command count from becoming an unbounded payload-memory or
upload-concurrency budget, while preserving the existing ABI and recovery
model.

## Plan

1. Give the Rust core one explicit owner for outgoing attachment limits,
   admitted bytes, and active request identifiers.
2. Acquire a non-cloneable permit before copying caller bytes and move it
   through the command queue, recovery deferral, upload task, and terminal event.
3. Map invalid and duplicate requests to `InvalidArgument`; map byte, count, and
   command pressure to the existing retryable `QueueFull` result.
4. Cover boundary, duplicate-ID, readiness, command-pressure, panic, and
   cancellation-drop paths with small deterministic tests.
5. Publish one attachment policy that distinguishes independent incoming,
   outgoing, event-queue, receive-prompt, and image-decoder budgets.

## Implemented invariants

- One file remains limited to 25 MiB.
- At most two outgoing files totaling 50 MiB are admitted per core.
- Admission spans queued, recovery-deferred, active, and completed-but-not-yet
  reported work, so it also caps concurrent uploads at two.
- Zero and duplicate active attachment request identifiers are rejected.
- Dropping a command, deferred item, task, completion, or core releases its
  request identifier and byte budget without a manual cleanup branch.
- ABI version 7 and the existing status/event vocabulary remain unchanged.

## Completed follow-up

The v0.3.0 reliability-hardening phase made cancellation independent of
command-channel capacity, retained C transfer ownership through cancellation,
and replaced the adapter's eager whole-file read with a bounded single-open
regular-file reader.
