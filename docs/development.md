# Development

## Toolchain

The target runtime is Debian 13 with libpurple 2.14.14. The current dependency
graph requires Rust 1.94 or later; `rust-toolchain.toml` pins 1.95.0.

Install system dependencies:

```sh
sudo apt install build-essential cmake ninja-build pkg-config \
  libpurple-dev libglib2.0-dev libsecret-1-dev libssl-dev \
  clang libclang-dev protobuf-compiler
rustup toolchain install 1.95.0 --component rustfmt,clippy
```

CDSI is intentionally disabled because its BoringSSL dependency conflicts with
the SQLCipher/OpenSSL build used here.

## Standard checks

```sh
cargo fmt --manifest-path rust/signal-core/Cargo.toml -- --check
cargo clippy --locked --manifest-path rust/signal-core/Cargo.toml \
  --all-targets -- -D warnings
cargo test --locked --manifest-path rust/signal-core/Cargo.toml --lib

cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Debug
cmake --build build
ctest --test-dir build --output-on-failure
```

The C tests include a headless libpurple core that probes and loads the actual
plugin module plus focused markup and contact-snapshot reconciliation. The
Rust tests cover owned ABI payloads, FFI error outputs, bounded event overflow,
QR PNG generation, and group-key validation. Live compatibility tests require dedicated
non-production Signal accounts and are not run for untrusted pull requests.

## C rules

- Define `PURPLE_PLUGINS` for all plugin translation units.
- Call Purple only on the GLib main thread.
- Track and destroy every source/request before freeing connection state.
- Treat Rust events as immutable and call `signal_event_free` exactly once.
- Strip outgoing markup and escape incoming remote text.

## Rust rules

- Keep Presage work on the backend actor's Tokio `LocalSet`.
- Validate every public ABI pointer, UTF-8 string, and length.
- Contain panics at exported boundaries and keep teardown non-panicking; never
  unwind into C.
- Never expose raw upstream `libsignal` bridge symbols.
- Keep `Cargo.lock` and exact Git revisions in reviewable commits.

## Updating dependencies

Follow [dependency-policy.md](dependency-policy.md). A build-only result is not
enough for Signal-stack changes.
