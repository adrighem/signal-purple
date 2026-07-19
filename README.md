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
| Signal contact buddy-list create/update/delete | Implemented; 46-contact create/refresh live-tested and snapshot reconciliation unit-tested |
| Group metadata synchronization | Implemented; live test pending |
| Plain-text direct messages | Live isolated-profile test passed 2026-07-19 |
| Basic group messages | Implemented; live test pending |
| Typing indicators | Implemented; live test pending |
| Delivery receipts | Sent by the backend; Purple 2 has no receipt UI |
| Incoming attachments | Shown as notices only |
| File transfer, safety-number verification, calls | Not implemented |
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
        │ owned polling ABI
        ▼
libsignal_core.so         Rust actor thread, Tokio LocalSet, SQLCipher
        │
        ▼
Presage → libsignal-service-rs → Signal libsignal
```

The Purple side never calls Signal libraries directly. Each account owns one
backend actor and polls owned events on the GLib main context. Details are in
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

- Unknown contact identities use trust-on-first-use; changed identity keys are
  rejected. This build has no per-contact acceptance workflow, so that contact
  remains blocked in the current store. Verifying in an official client does
  not update signal-purple's separate linked-device store. Pinned Presage can
  skip an inbound identity error before this adapter sees it, so an affected
  inbound message may disappear without a Purple warning; outbound sends fail
  with a generic operation error.
- Attachment contents are not downloaded or uploaded. Incoming attachment
  names are rendered as notices.
- Synchronized groups open when their first message arrives. Proactively
  browsing or opening a group is not implemented. Closing a Purple chat never
  leaves the Signal group.
- Synchronized contacts are projected into Purple's `Signal` buddy-list group.
  Each connection requests a fresh contact sync from the primary phone.
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
  still stores contact aliases and stable Signal identifiers in plaintext in
  `blist.xml`; another UI or plugin could retain additional plaintext.
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
