# Track Specification: Modular Backend and FFI Refactor

## Overview

Deconstruct the monolithic `backend.rs` (6,580 lines) into focused, single-responsibility submodules under `rust/signal-core/src/backend/`, and optimize attachment FFI transfers to stream directly from disk rather than copying full file buffers.

## Background

The Rust core of `signal-purple` concentrates nearly all protocol runtime logic inside `rust/signal-core/src/backend.rs`. This single file handles:
- Avatar image downscaling and caching
- Sandboxed external process execution (`prlimit`/`ffmpeg` GIF transcoding)
- Durable outbox polling and exponential retry loops
- Incoming message projection, deduplication, and acknowledgment draining
- Connection lifecycle, link QR handling, and Tokio worker orchestration

In addition, outgoing attachment handling in `ffi.rs` copies up to 25 MiB of raw data from C buffers into Rust memory heaps, causing memory spikes and duplicate allocations during file transfers.

## Functional Requirements

- Break down `backend.rs` into specialized submodules:
  - `backend/media.rs`: Avatar downscaling and GIF transcoding routines.
  - `backend/outbox.rs`: Client outbox worker, message dispatch, and retry scheduling.
  - `backend/projection.rs`: Inbound message projection, delivery receipts, and acknowledgment draining.
  - `backend/worker.rs`: Main Tokio runtime coordination, command dispatch, and shutdown signaling.
- Maintain existing module exports and visibility so that external consumers and tests continue to compile without disruption.
- Stream large outgoing attachments directly from file descriptors or file paths across FFI to eliminate double-buffering.

## Non-Functional Requirements

- Preserve ABI version 7 and existing `SignalEvent` / `SignalStatus` semantics.
- Adhere to the Single Responsibility Principle and file size guidelines (< 500 lines per module where practical).
- Strict type hints, no trailing spaces in files, and zero emdash usage.
- Retain all panic guards (`ffi_guard`) across exported FFI boundaries.
- Pass all unit tests and `scripts/check.sh full`.

## Acceptance Criteria

- `backend.rs` serves as a clean aggregator/facade re-exporting submodules.
- Each extracted submodule has isolated unit test coverage.
- Outgoing attachment transfers consume bounded memory without copying full payload buffers across FFI before transmission.
- All existing ABI tests, CTest validation gates, and Rust test suites pass cleanly.

## Out of Scope

- Changing libpurple 2 C plugin architecture or replacing GLib main loop.
- Modifying Signal protocol wire formats or Presage cryptographic primitives.
- Upgrading to ABI version 8.
