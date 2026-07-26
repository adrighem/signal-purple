# Development

## Toolchain

The target runtime is Debian 13 with libpurple 2.14.14. The current dependency
graph requires Rust 1.94 or later; `rust-toolchain.toml` pins 1.95.0.

Install system dependencies:

```sh
sudo apt install build-essential cmake git gnupg ninja-build pkg-config python3 xz-utils \
  libpurple-dev libglib2.0-dev libgdk-pixbuf-2.0-dev libsecret-1-dev libssl-dev \
  clang libclang-dev protobuf-compiler
rustup toolchain install 1.95.0 --component rustfmt,clippy
```

CDSI is intentionally disabled because its BoringSSL dependency conflicts with
the SQLCipher/OpenSSL build used here.

## Standard checks

Use the canonical fast feedback loop while changing Rust code:

```sh
scripts/check.sh fast
```

Before requesting review, run the same complete local gate used by the primary
CI job:

```sh
scripts/check.sh full
```

The full gate runs Rust format, clippy, and all Rust tests; release-helper
tests; a Debug CMake/Ninja build; CTest; and a staged installation/plugin-load
probe. Set `SIGNAL_PURPLE_BUILD_DIR` to use a different build directory and
`SIGNAL_PURPLE_BUILD_JOBS` to change build parallelism. The separate Debian 13
CI job validates the clean supported-platform environment.

Each phase is also directly runnable, for example:

```sh
scripts/check.sh rust-test
scripts/check.sh configure
scripts/check.sh build
scripts/check.sh c-test
ctest --test-dir build --output-on-failure --no-tests=error -R contact-sync
cargo test --locked --manifest-path rust/signal-core/Cargo.toml \
  bounds_binary_events
```

The C tests include a headless libpurple core that probes and loads the actual
plugin module plus focused ABI values, conversation logging, outgoing-transfer
lifetime, bounded file admission, markup, inline-image ownership and routing,
and contact-snapshot reconciliation. The Rust tests cover ABI values and owned
payloads, FFI error outputs, acknowledgement and cancellation pressure,
timestamp allocation, bounded shutdown, credential lifetime, event overflow,
QR PNG generation, group-key validation, and group-image projection. The
staged-install probe also verifies the Linux backend's ELF `NODELETE` contract.
Live compatibility tests require dedicated non-production Signal accounts and
are not run for untrusted pull requests.

## C rules

- Define `PURPLE_PLUGINS` for all plugin translation units.
- Call Purple only on the GLib main thread.
- Track and destroy every source/request before freeing connection state.
- Register outgoing transfers at creation and detach their contexts before
  freeing connection state, including transfers still awaiting file selection.
- Hold a temporary transfer reference across `purple_xfer_start`, which may
  synchronously cancel, and cancel every started transfer during disconnect.
- Admit local attachment bytes through the bounded, single-open regular-file
  reader rather than an unbounded path-based convenience API.
- Treat Rust events as immutable and call `signal_event_free` exactly once.
- Strip outgoing markup and escape incoming remote text.

## Rust rules

- Keep Presage work on the backend actor's Tokio `LocalSet`.
- Validate every public ABI pointer, UTF-8 string, and length.
- Contain panics at exported boundaries and keep teardown non-panicking; never
  unwind into C.
- Never expose raw upstream `libsignal` bridge symbols.
- Keep `Cargo.lock` and exact Git revisions in reviewable commits.
- Keep bounded producer/consumer mechanics in `event_queue.rs`; the backend
  actor emits events and the FFI layer polls them rather than reimplementing
  notification state.
- Keep correctness-critical control paths out of best-effort work admission:
  projection acknowledgements coalesce by registered delivery ID and attachment
  cancellation lives on the admission permit.
- Allocate adapter-generated protocol timestamps through the shared per-core
  allocator. Keep outbox retry deadlines on the wall clock and preserve the
  original timestamp when retransmitting a durable message.
- Race network-backed profile, synchronization, replay, outbox, and projection
  phases, plus store registration and initialization, against worker shutdown.
  Complete blocking filesystem setup before creating the worker so it cannot
  outlive teardown. Dropping a projection must leave it eligible for replay,
  and dropping an outbox attempt must leave its encrypted row for a later retry.
  Keep recovery cleanup and Tokio runtime shutdown bounded; undrained
  projection acknowledgements rely on the same durable replay path. SQLx
  SQLite workers are dependency-owned and unjoinable, so Linux builds must keep
  the Rust backend process-resident with ELF `NODELETE`.
- Copy passphrases from the C ABI directly into `StorePassphrase`. Move that
  owner into the store-opening boundary and wipe it before returning the store
  or an error; never retain it in session configuration.

## Updating dependencies

Follow [dependency-policy.md](dependency-policy.md). A build-only result is not
enough for Signal-stack changes.
