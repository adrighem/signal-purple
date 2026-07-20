# signal-purple

> **Pre-alpha and unofficial.** This project is not affiliated with or
> supported by Signal. Only the scenarios listed in
> [live validation](docs/live-validation.md) have been tested against the
> production service. Do not treat this as a substitute for an official Signal
> client.

`signal-purple` is an experimental Signal protocol plugin for Pidgin and other
libpurple 2 clients. It links as a secondary Signal device, keeps Signal
protocol work in a pinned Rust/Presage backend, and exposes a small owned C ABI
to the Purple plugin.

## Current status

| Capability | Status |
| --- | --- |
| Purple 2.14 plugin discovery/load | Implemented and tested |
| Fresh-store linked-device QR flow | Live isolated-profile test passed 2026-07-19 |
| Existing linked-device reconnect | Live isolated-profile test passed 2026-07-19 |
| SQLCipher store with libsecret key | Implemented |
| Signal contact buddy-list create/update/delete | Implemented; 46-contact creation and 49-contact legacy migration live-tested; snapshot reconciliation unit-tested |
| Group discovery, active-membership reconciliation, and pruning | Implemented; earlier 11-group discovery and 3-member projection live-tested, authoritative refresh live test pending |
| Plain-text direct messages | Live isolated-profile test passed 2026-07-19 |
| Basic group messages | Implemented for refreshed active memberships; live send test pending |
| Remote group leave | Implementation-tested; production-service validation pending |
| Typing indicators | Implemented; live test pending |
| Delivery receipts | Sent by the backend; Purple 2 has no receipt UI |
| Incoming and outgoing attachments | Implemented with 25 MiB per-file and bounded-memory limits; live test pending |
| In-plugin safety-number comparison, calls | Not implemented |
| Primary-device registration and contact discovery | Out of scope for now |

Signal does not provide a stable third-party client API. Service changes can
break this plugin without warning. See [compatibility](docs/compatibility.md)
and the [security model](docs/security-model.md) before using it.

## Architecture

```text
Pidgin / libpurple 2
        │
        ▼
libsignal-purple.so       C, GLib main thread, Purple lifecycle and UI
        │ owned event ABI + private wake descriptor
        ▼
libsignal_core.so         Rust actor thread, Tokio LocalSet, SQLCipher
        │
        ▼
Presage → libsignal-service-rs → Signal libsignal
```

The Purple side never calls Signal libraries directly. Each account owns one
backend actor; a private nonblocking descriptor wakes the GLib main context to
drain owned events without an idle polling timer. Details are in
[architecture.md](docs/architecture.md).

## Requirements

The supported baseline is Debian 13 with libpurple 2.14.14. Building requires:

```sh
sudo apt install build-essential cmake ninja-build pkg-config \
  libpurple-dev libglib2.0-dev libsecret-1-dev libssl-dev \
  clang libclang-dev protobuf-compiler
```

The pinned Presage SQLite stack currently requires Rust 1.94 or later. The
repository pins Rust 1.95.0; install it with rustup when the distro compiler is
older:

```sh
rustup toolchain install 1.95.0 --component rustfmt,clippy
```

## Build and test

```sh
cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_INSTALL_PREFIX=/usr
cmake --build build
ctest --test-dir build --output-on-failure
cargo test --locked --manifest-path rust/signal-core/Cargo.toml
```

The first build downloads exact Git revisions recorded in
[`Cargo.lock`](rust/signal-core/Cargo.lock). Release source bundles will need to
vendor these dependencies before Debian can build them without network access.

Install system-wide with:

```sh
sudo cmake --install build
```

For a per-user test install, configure a separate build so CMake writes the
correct relative runtime path:

```sh
cmake -S . -B build-user -G Ninja -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_INSTALL_PREFIX="$HOME/.purple" \
  -DSIGNAL_PURPLE_PLUGIN_DIR=plugins \
  -DSIGNAL_CORE_INSTALL_DIR=plugins/signal-purple
cmake --build build-user
cmake --install build-user
```

The private backend subdirectory prevents Purple from probing
`libsignal_core.so` as if it were another plugin.

## Link an account

1. In Pidgin, choose **Accounts → Manage Accounts → Add**.
2. Select **Signal** and enter any local account label. No Signal password is
   stored in Purple.
3. Enable the account. For a new store, the plugin displays a QR code.
4. On the primary phone, open **Signal Settings → Linked devices → Link new
   device** and scan the QR code.
5. Wait until pending Signal updates are synchronized. Only then does Purple
   mark the account connected and allow sends.

The encrypted database defaults to
`~/.purple/signal-purple/<account-hash>.db3`. Its randomly generated passphrase
is stored in the desktop secret service through libsecret. Losing either the
database or secret requires linking a new device.

## Important limitations

- Unknown contact identities use trust-on-first-use. A changed identity for an
  unverified contact continues after a one-time warning. A changed identity for
  an explicitly verified contact blocks sending until the buddy-menu acceptance
  action is used. Acceptance requires out-of-band verification, resets affected
  sessions, and marks the contact unverified without relinking the account.
- Direct and group attachments use Purple's native file-transfer UI. Files are
  limited to 25 MiB each and incoming messages to 50 MiB total. Decrypted
  incoming data stays in memory while Purple asks for a save location; no
  plaintext attachment cache is created. At most 64 MiB may wait in the backend
  event queue and another 64 MiB in unresolved receive prompts. Excess data is
  rejected visibly. Outgoing attachment uploads can be cancelled, but unlike
  text messages they are not retained in the restart-persistent outbox.
- Signal Storage Service groups are projected into Purple's localized default
  chat-list group (normally `Chats`). A complete refresh validates every group
  record returned for the current Storage Service manifest, then checks both
  discovered and previously cached groups against current GroupsV2 state before
  publishing an authoritative active-group snapshot. Only groups which Signal
  confirms are inaccessible or no longer contain this account are pruned from
  the encrypted cache and managed Purple chat list. An incomplete refresh does
  not trigger destructive pruning. Group joins and sends are limited to that
  refreshed active set; a failed refresh is retried in-session while group
  operations remain unavailable.
  Purple uses the stable opaque group identifier as the conversation identity,
  while the Signal title remains presentation data; a user-set local chat alias
  is preserved across title refreshes. Existing managed chats in the legacy
  `Signal groups` group move to the default group on sync, while custom
  placement and user-created chats are preserved.
- To leave an active group remotely, right-click its managed chat and choose
  **Leave Signal group…**, then confirm. The plugin removes the managed local
  chat only after Signal confirms the leave. Purple 2 does not provide protocol
  plugins with a callback for its built-in **Remove Chat** operation, so that
  action remains local-only and the chat can return after synchronization.
  Closing a conversation tab is also local-only and never leaves the group.
- Synchronized contacts are projected into Purple's localized default buddy
  group (for example, `Friends` or `Buddies`). Each connection requests a fresh
  contact sync from the primary phone. Existing contacts in the exact legacy
  `Signal` group are adopted only when the current Signal snapshot confirms
  their account and identifier, then move to the default group. Contact aliases,
  locally merged buddies, and custom placement are preserved.
  Complete snapshots create missing buddies, update server aliases, and remove
  previously synchronized contacts that are no longer present. Local aliases
  remain local. A synchronized phone number is the display fallback when no
  contact name is available. Signal does not expose presence, so synchronized
  contacts are shown as reachable while the account is connected. This does
  not provide phone-number discovery or mutate Signal's remote contact data.
- Message formatting is reduced to plain text. Incoming text is escaped before
  entering Purple; outgoing Purple markup is stripped.
- Reconnecting drains messages queued by Signal for this linked device until
  Presage reports `QueueEmpty`. The plugin cannot request arbitrary older
  conversation history from the phone or service, and it does not import an
  official client's local message database.
- Disappearing-message timers and remote deletions are not projected into
  Purple. The plugin disables logging for every Signal conversation, but Purple
  still stores contact aliases, group titles, and stable opaque identifiers in
  plaintext in `blist.xml`; raw Signal group master keys remain confined to the
  encrypted backend store. Another UI or plugin could retain additional
  plaintext.
- QR provisioning URIs are sensitive. The backend compiles out upstream
  info-level tracing so Presage's provisioning message cannot log the URI.

## Documentation

- [Development guide](docs/development.md)
- [Architecture](docs/architecture.md)
- [Security model](docs/security-model.md)
- [Compatibility policy](docs/compatibility.md)
- [Live validation](docs/live-validation.md)
- [Troubleshooting](docs/troubleshooting.md)
- [Roadmap](ROADMAP.md)
- [Licensing](docs/licensing.md)

Please use [GitHub issues](https://github.com/adrighem/signal-purple/issues) for
ordinary bugs and follow [SECURITY.md](SECURITY.md) for vulnerabilities.

## License and trademarks

Original C plugin code and general project material are licensed under
GPL-3.0-or-later. The Rust backend is AGPL-3.0-only because Presage and its
Signal stack use that license; AGPL terms apply to the combined binaries. See
[THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md) for exact pinned sources.

Signal, the Signal logo, and related names are trademarks of Signal Technology
Foundation. This project uses its own icon and is not endorsed by Signal.
