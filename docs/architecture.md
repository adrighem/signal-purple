# Architecture

signal-purple deliberately owns the boundary between Purple and the evolving
Signal client ecosystem.

## Components

`libsignal-purple.so` is a Purple 2 protocol plugin written in C. It owns Purple
accounts, conversations, buddy/group mapping, QR presentation, libsecret access,
and all GLib lifecycle work. It never performs Signal cryptography.

`libsignal_core.so` is a Rust `cdylib`. Each Purple account creates one opaque
core with a dedicated OS thread, a current-thread Tokio runtime, and a
`LocalSet`, matching Presage's non-`Send` runtime constraints. It owns network,
crypto, storage, linking, sync, and message normalization.

Presage, libsignal-service-rs, and libsignal are exact Git revisions recorded in
the lockfile. Purple never calls their unstable interfaces directly.

## ABI and ownership

[`include/signal_core.h`](../include/signal_core.h) is the only C/Rust contract.
It is versioned and exposes opaque cores, asynchronous commands, polling, and
explicit event destruction.

- C strings passed into a command are validated and copied before return.
- Rust owns all event strings and blobs until `signal_event_free`.
- The event queue is bounded at 4096 entries. Overflow produces a fatal event
  and requires reconnecting so data is resynchronized instead of silently
  dropping an arbitrary message.
- Fallible exported operations catch panics at the FFI boundary. Teardown is
  deliberately written from non-panicking primitives so the worker is always
  joined before its allocation is freed.
- The C plugin polls on an explicitly owned GLib timeout source.
- Teardown destroys the polling source, sends shutdown, joins the worker, then
  frees the core. No worker calls into C or Purple.

## Connection sequence

1. Purple resolves an account-specific database path.
2. The plugin loads or generates its SQLCipher passphrase through libsecret.
3. The Rust core opens the database.
4. An existing linked device loads immediately. A fresh store starts Presage's
   secondary-device provisioning and emits a QR PNG.
5. The backend starts the receive stream and processes queued sync/session data.
6. Only Presage's first `QueueEmpty` event marks the core ready and the Purple
   account connected.

This ordering prevents sends before queued profile, session, and sender-key
updates have been applied.

## Message mapping

- Canonical Signal service identifiers are Purple buddy names. Synced profile
  names are aliases only.
- Group master keys remain private 32-byte backend identifiers, represented as
  64 hex characters only across the internal ABI. They are never used as
  Purple titles or join metadata. Each connection assigns a collision-free
  sequential Purple chat integer.
- Incoming text is markup-escaped. Outgoing Purple markup is stripped.
- Own-device `SynchronizeMessage` values render as outgoing messages.
- Delivery receipts are sent when Presage marks an envelope as needing one.
- Purple 2 has no robust per-message receipt update API, so received receipts
  are currently consumed without a misleading UI projection.

## Deliberate boundaries

The first version does not implement attachment transfers, safety-number state
changes, primary registration, contact discovery, calls, or official backup
compatibility. These need separate designs rather than thin callback additions.
It also does not project disappearing timers or remote deletion into Purple.
The adapter disables conversation logging and marks delivered messages no-log,
but synced buddy aliases and identifiers still live in Purple's plaintext buddy
list.
