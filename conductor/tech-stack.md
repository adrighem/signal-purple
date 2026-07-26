# Tech Stack

## Languages

- C11 for the Purple adapter.
- Rust 2024 for the Signal core.
- POSIX shell and Python for build and release helpers.

## Frameworks and Libraries

- libpurple 2.14, GLib/GIO, GdkPixbuf, and libsecret.
- Tokio current-thread runtime and `LocalSet`.
- Presage and libsignal dependencies pinned to exact Git revisions.

## Data Stores

- SQLCipher-backed SQLite through the pinned Presage store.
- Desktop Secret Service for the database passphrase.
- Bounded in-memory command and event queues.

## Tooling

- CMake 3.25+, Ninja, Cargo, rustfmt, Clippy, and CTest.
- `scripts/check.sh full` is the canonical local validation gate.

## Constraints and Decisions

- Debian 13 and Rust 1.95.0 are the supported build baseline.
- Purple calls remain on the GLib main thread.
- Presage work remains on the Rust backend actor.
- The C/Rust boundary uses owned values and opaque handles only.
- Tests run under a minimal allowlisted environment so inherited credentials
  cannot enter output or artifacts.
