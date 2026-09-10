# Track Plan: SQLite Persistence Hardening

## Phase 1: Structured Error Classification

- [x] Task: Replace string-based error matching with structured SQLite error codes 4490fc1
  - [x] Sub-task: Add regression unit tests simulating database busy and lock conditions
  - [x] Sub-task: Match on `sqlx::Error::Database` / `libsqlite3-sys` error codes (e.g., `SQLITE_BUSY`, `SQLITE_LOCKED`, `SQLITE_BUSY_TIMEOUT`) in `sqlite_store_error_is_transient`
  - [x] Sub-task: Propagate structured error inspection to `signal_protocol_error_is_transient`
  - [x] Sub-task: Verify all existing transient error recovery and backoff unit tests pass

## Phase 2: In-Memory Metadata Caching

- [x] Task: Introduce in-memory metadata caching for read paths 35d4348
  - [x] Sub-task: Design thread-safe in-memory cache for group revision numbers and contact names
  - [x] Sub-task: Route group metadata queries through cache before touching `SqliteStore`
  - [x] Sub-task: Implement cache invalidation hooks on contact sync end and group revision changes
  - [x] Sub-task: Add unit tests verifying cache hit paths and invalidation consistency

## Phase 3: Storage Repository Abstraction

- [x] Task: Encapsulate database interactions in a Storage Repository module 2f1302c
  - [x] Sub-task: Create `rust/signal-core/src/store/repository.rs` defining clean methods for outbox, projection, and identity queries
  - [x] Sub-task: Move raw SQL/Presage store invocations out of `backend.rs` into the repository
  - [x] Sub-task: Update backend worker loops to interact exclusively via the repository
  - [x] Sub-task: Run full check suite (`scripts/check.sh`) to verify regressions
