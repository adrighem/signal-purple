# Track Plan: Modular Backend and FFI Refactor

## Phase 1: Module Decomposition

- [x] Task: Extract media processing and GIF transcoding (fbbd2b2)
  - [x] Sub-task: Create `rust/signal-core/src/backend/media.rs` containing avatar downscaling, disk caching, and `ffmpeg` execution
  - [x] Sub-task: Move associated media tests to `media.rs` and verify clean compilation
- [x] Task: Extract client outbox and retry loop (ad83bb7)
  - [x] Sub-task: Create `rust/signal-core/src/backend/outbox.rs` containing outbox polling, backoff logic, and message expediting
  - [x] Sub-task: Verify outbox retry unit tests pass in isolation
- [x] Task: Extract message projection and acknowledgment tracking (d67c4d4)
  - [x] Sub-task: Create `rust/signal-core/src/backend/projection.rs` containing `MessageProjection`, delivery receipt handling, and replay queue
  - [x] Sub-task: Verify projection deduplication and acknowledgment tests pass
- [x] Task: Re-organize backend coordinator and worker loop (3e93d98)
  - [x] Sub-task: Establish `rust/signal-core/src/backend/worker.rs` for Tokio task scheduling, command routing, and shutdown signals
  - [x] Sub-task: Keep `rust/signal-core/src/backend.rs` (or `backend/mod.rs`) as a clean facade exporting public types

## Phase 2: Attachment FFI Streaming Optimization

- [x] Task: Stream attachments across FFI boundary without duplicate buffering
  - [x] Sub-task: Add path/file descriptor attachment transmission variant in `rust/signal-core/src/ffi.rs`
  - [x] Sub-task: Stream data directly from disk using Tokio asynchronous file IO
  - [x] Sub-task: Update C caller in `src/connection.c` to use streaming path
  - [x] Sub-task: Add regression test confirming bounded memory footprint on 25 MiB attachment transfer
