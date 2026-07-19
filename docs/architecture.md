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
6. At the first `QueueEmpty`, the backend reads the account's Storage Service
   manifest and refreshes every available group into the encrypted local store.
7. Only after contact and group snapshots are emitted does the core become ready
   and the Purple
   account connected.

This ordering prevents sends before queued profile, session, and sender-key
updates have been applied. The queue contains envelopes addressed to this
linked device, including messages Signal still has queued after it was offline.
It is not a general conversation-history API and cannot retrieve arbitrary
older messages from the primary phone or Signal service.

## Message mapping

- Canonical Signal service identifiers are Purple buddy names. Synced profile
  names are server aliases only. Explicit snapshot boundaries let Purple apply
  contact creates and updates before removing stale managed entries. User-made
  buddies without the managed marker are never swept. The backend explicitly
  requests a contact sync after opening the receive stream, then refreshes the
  projection when Presage reports synchronized contacts. Because Signal does
  not expose presence, contacts are marked reachable while the linked account
  is connected so Purple's default offline filter does not hide them.
- Group master keys remain private 32-byte values in the encrypted backend
  store. Purple receives a domain-separated SHA-256 identifier for persistence
  and joining, then Rust resolves that identifier back to a stored group when
  sending. Snapshot reconciliation updates titles and active membership,
  removes stale plugin-managed entries, and collapses duplicate managed chats.
  Each connection assigns a collision-free sequential Purple chat integer.
- Incoming text is markup-escaped. Outgoing Purple markup is stripped.
- Own-device `SynchronizeMessage` values render as outgoing messages.
- Delivery receipts are sent when Presage marks an envelope as needing one.
- Purple 2 has no robust per-message receipt update API, so received receipts
  are currently consumed without a misleading UI projection.

## Flare comparison

Flare uses the same pinned Presage revision but presents contacts and groups as
conversation threads instead of a presence-oriented buddy list. Its UI exposes
a manual
[`sync-contacts` action](https://gitlab.com/schmiddi-on-mobile/flare/-/blob/484450e4cf8a34992a68df753a872e530a5b3d2c/src/gui/window.rs#L353)
that delegates to Presage's contact request.
[After the receive queue is empty](https://gitlab.com/schmiddi-on-mobile/flare/-/blob/484450e4cf8a34992a68df753a872e530a5b3d2c/src/backend/manager.rs#L106),
Flare initializes channels from its local thread store. Its
[`contacts()` projection](https://gitlab.com/schmiddi-on-mobile/flare-backend/-/blob/8f9f178cb5ec9040d73fdd7c70a3ca3a5bcdcb72/flare-store/src/lib.rs#L133)
also enriches synchronized contact names with stored Signal profiles.

signal-purple applies the same essential contact-request step automatically on
every connection. It then reconciles complete snapshots into plugin-managed
Purple buddies. Because Purple normally hides offline buddies and Signal has no
presence API, synchronized contacts are marked reachable while the account is
connected. Contact names and synchronized phone numbers are used as aliases;
profile enrichment is not implemented yet. For groups, signal-purple's pinned
Presage fork adds a read-only Storage Service synchronization method so the chat
list is complete without waiting for each group to receive a new message.

## Deliberate boundaries

The first version does not implement attachment transfers, safety-number state
changes, primary registration, contact discovery, calls, or official backup
compatibility. These need separate designs rather than thin callback additions.
It also does not project disappearing timers or remote deletion into Purple.
The adapter disables conversation logging and marks delivered messages no-log,
but synced buddy aliases and identifiers still live in Purple's plaintext buddy
list.
