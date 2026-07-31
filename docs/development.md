# Development

## Toolchain

The target runtime is Debian 13 with libpurple 2.14.14. The current dependency
graph requires Rust 1.94 or later; `rust-toolchain.toml` pins 1.95.0.

Install system dependencies:

```sh
sudo apt install build-essential clang-format-19 cmake ffmpeg git gnupg \
  ninja-build pkg-config python3 util-linux xz-utils \
  libpurple-dev libglib2.0-dev libgdk-pixbuf-2.0-dev libsecret-1-dev libssl-dev \
  clang libclang-dev protobuf-compiler
rustup toolchain install 1.95.0 --component rustfmt,clippy
```

CDSI is intentionally disabled because its BoringSSL dependency conflicts with
the SQLCipher/OpenSSL build used here.

`ffmpeg` and `prlimit` from `util-linux` are optional at runtime. When both are
available at their Debian paths, the receive worker can convert strictly
eligible Signal GIF-style MP4 attachments to bounded inline GIFs. Missing
helpers leave the original receive-file behavior intact.

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

The full gate runs the pinned C formatter, Rust format, clippy, and all Rust
tests; release-helper tests; a Debug CMake/Ninja build; CTest; and a staged
installation/plugin-load probe. Set `SIGNAL_PURPLE_BUILD_DIR` to select the
build directory and `SIGNAL_PURPLE_BUILD_JOBS` to change build parallelism. The
separate Debian 13 CI job validates the clean supported-platform environment.

Each phase is also directly runnable, for example:

```sh
scripts/check.sh rust-test
scripts/check.sh c-format
scripts/check.sh c-format-fix
scripts/check.sh configure
scripts/check.sh build
scripts/check.sh c-test
ctest --test-dir build --output-on-failure --no-tests=error -R contact-sync
cargo test --locked --manifest-path rust/signal-core/Cargo.toml \
  bounds_binary_events
```

The C gate requires clang-format 19 and rejects any other major version. Its
repository-owned style preserves intentional line wrapping while normalizing
indentation, declarations, braces, and token spacing. Use `c-format-fix` to
apply that exact style.

The C tests include a linked C/Rust ABI constant, layout, and input-limit check
plus a headless libpurple core that probes and loads the actual plugin module,
conversation logging, outgoing-transfer lifetime, bounded file admission,
markup, inline-image ownership and routing, and contact-snapshot reconciliation.
A relocation check proves that helpers
compiled into the export-dynamic probe executable cannot preempt the loaded
module's own implementations, and every direct probe invocation has a hard
timeout. The Rust tests cover ABI values and owned payloads, FFI error outputs,
acknowledgement and cancellation pressure, timestamp allocation, bounded
shutdown, credential lifetime, event overflow, QR PNG generation, group-key
validation, group-image projection, and optional bounded Signal GIF-style
conversion. The staged-install probe also verifies the Linux backend's ELF
`NODELETE` contract.
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
- Keep the pinned Presage SQLx pool at one connection. Signal protocol
  read-modify-write transactions must serialize at the database boundary;
  retrying an entire send after a store error can duplicate its remote effect.
- Keep the Presage receive stream independently scheduled on the actor's
  `LocalSet` and forward only through a bounded ordered channel. Do not poll the
  stream directly in a `select!` whose winning branches await other store work:
  an in-progress stream future can own the sole connection and must remain
  polled to release it.
- Validate every public ABI pointer, UTF-8 string, and length.
- Keep every public input bound in `signal_core.h` and the compiled ABI
  conformance manifest; changing either side must fail the linked C test.
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
