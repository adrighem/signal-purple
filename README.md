# signal-purple

> [!WARNING]
> **Unofficial and not supported by Signal.** Expect Signal service
> changes to break compatibility without warning.

`signal-purple` is a Signal linked-device protocol plugin for Pidgin and
libpurple 2. It adds synchronized Signal contacts, direct messages, and group
conversations to Pidgin while a Rust/Presage backend handles the Signal
protocol inside the same process.

Published 1.x releases are stable within the documented supported environment
and feature scope. Stability covers the project's release and support contract;
it does not imply Signal endorsement, full feature parity, guaranteed
compatibility with future Signal service changes, or an independent security
audit.

## Current state

This README distinguishes production-service evidence from automated tests:

- **Live-tested** means the behavior was exercised against Signal's production
  service in a controlled Pidgin profile.
- **Test-covered** means the current implementation is exercised by automated
  tests, but still needs production-service validation.
- **Implemented** means the code path exists and has focused coverage, but the
  complete user-visible flow has not yet been validated end to end.

| Area | Current evidence |
| --- | --- |
| Linked-device QR setup and encrypted-store reconnect | **Live-tested.** A fresh link and a restart without relinking both passed. |
| Direct plain-text messages | **Live-tested** in both directions. |
| Contacts | **Partly live-tested.** Contact creation and adoption from the legacy `Signal` buddy group passed live checks; update and removal snapshot decisions are test-covered. |
| Group discovery and display | **Partly live-tested.** Discovery, reconnect deduplication, titles, members, and administrator flags passed live checks. Current authoritative refresh and pruning are test-covered. |
| Group messages | **Implemented.** Routing and active-membership guards have automated coverage; current end-to-end send/receive still needs live validation. |
| Remote group leave | **Test-covered.** The confirmed **Leave Signal group…** flow still needs live validation. |
| Typing and receipts | **Test-covered.** Direct typing and outgoing delivery/focus-gated read receipts are implemented. Receipt updates received from other clients are not shown in Purple 2. |
| Attachments | **Implemented.** Size limits, ABI handling, transfer presentation, and direct/group inline-image routing have focused tests. End-to-end transfers and inline media still need live checks. |
| Delivery recovery | **Test-covered.** The encrypted text outbox and unacknowledged-message replay are implemented; controlled offline, crash, and network-loss checks remain. |
| Identity replacement | **Test-covered.** Verified-contact sends block until the user accepts a changed identity after out-of-band verification. |
| Idle event handling | **Live-tested.** Descriptor-driven wakeups replaced the old polling loop; an isolated idle sample found no hot Signal thread. |

Production-service evidence is revision-specific. See
[live validation](docs/live-validation.md) for the exact scenarios, revisions,
and outstanding checks. Stable status and passing automated tests do not
establish general Signal compatibility.

## Supported environment

The supported baseline is:

- Debian 13 or Ubuntu 24.04 LTS with Pidgin and libpurple 2.14.x;
- a desktop Secret Service provider accessible through libsecret;
- an existing Signal account on a current official Android or iOS client; and
- Rust 1.94 or newer to build the plugin (the repository pins Rust 1.95.0).

Another Purple 2 UI may work if it supports the request fields, transfers, and
image store used by the plugin, but only Pidgin is tested. Purple 3 and other
operating-system baselines are not supported. See the full
[compatibility policy](docs/compatibility.md).

## Build and install

### Install from the APT repository

Stable releases are available for Debian 13 and Ubuntu 24.04 LTS on amd64.
Install the repository's dedicated signing key:

```sh
sudo apt install ca-certificates curl
sudo install -d -m 0755 /etc/apt/keyrings
keyring_tmp="$(mktemp)"
curl --fail --silent --show-error --location \
  https://adrighem.github.io/signal-purple/apt/signal-purple-archive-keyring.gpg \
  --output "$keyring_tmp" || { rm -f "$keyring_tmp"; exit 1; }
sudo install -m 0644 "$keyring_tmp" \
  /etc/apt/keyrings/signal-purple-archive-keyring.gpg
rm -f "$keyring_tmp"
```

Then install the source definition matching the system.

Debian 13:

```sh
sources_tmp="$(mktemp)"
curl --fail --silent --show-error --location \
  https://adrighem.github.io/signal-purple/apt/signal-purple-debian-13.sources \
  --output "$sources_tmp" || { rm -f "$sources_tmp"; exit 1; }
sudo install -m 0644 "$sources_tmp" \
  /etc/apt/sources.list.d/signal-purple.sources
rm -f "$sources_tmp"
```

Ubuntu 24.04 LTS:

```sh
sources_tmp="$(mktemp)"
curl --fail --silent --show-error --location \
  https://adrighem.github.io/signal-purple/apt/signal-purple-ubuntu-24.04.sources \
  --output "$sources_tmp" || { rm -f "$sources_tmp"; exit 1; }
sudo install -m 0644 "$sources_tmp" \
  /etc/apt/sources.list.d/signal-purple.sources
rm -f "$sources_tmp"
```

Install signal-purple:

```sh
sudo apt update
sudo apt install signal-purple
```

The repository retains up to two stable package versions: the current release
and its stable predecessor when one exists. A normal `sudo apt update` followed
by `sudo apt upgrade` installs an update when one is published. Close Pidgin
before upgrading, then follow the [upgrade checks](#upgrade).

### Dependencies

On Debian 13 or Ubuntu 24.04 LTS:

```sh
sudo apt install pidgin ffmpeg git gnupg build-essential cmake ninja-build pkg-config python3 \
  libpurple-dev libglib2.0-dev libgdk-pixbuf-2.0-dev libsecret-1-dev \
  libssl-dev clang libclang-dev protobuf-compiler util-linux xz-utils
```

Install [rustup](https://rustup.rs/), then install the pinned toolchain when the
system compiler is older:

```sh
rustup toolchain install 1.95.0 --component rustfmt,clippy
```

### Build and test

```sh
git clone https://github.com/adrighem/signal-purple.git
cd signal-purple || exit

cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Release \
  -DBUILD_TESTING=ON \
  -DCMAKE_INSTALL_PREFIX=/usr
cmake --build build
ctest --test-dir build --output-on-failure --no-tests=error
cargo test --locked --manifest-path rust/signal-core/Cargo.toml
```

A contributor should run the canonical `scripts/check.sh full` gate before
requesting review; it also checks formatting, linting, release helpers, and a
staged plugin installation. See the [development guide](docs/development.md)
for faster targeted modes.

A normal checkout downloads the exact Git dependencies recorded in
[`Cargo.lock`](rust/signal-core/Cargo.lock) on its first build. Source archives
generated by [`scripts/make-source-archive.sh`](scripts/make-source-archive.sh)
vendor the locked dependency graph for offline Debian builds; see the
[Debian packaging guide](docs/debian-packaging.md).

### Install system-wide

Fully quit Pidgin, including any process left in the notification area, then
install both shared libraries from the same build:

```sh
sudo cmake --install build
```

Restart Pidgin and confirm that **Signal** appears under **Accounts → Manage
Accounts → Add**. Never upgrade only `libsignal-purple.so` or only
`libsignal_core.so`; their private ABI must match. A stale per-user plugin can
also shadow a system installation, so use one installation scope at a time. A
CMake source install has no automated uninstall target; keep
`build/install_manifest.txt` and follow the documented
[installation lifecycle](#installation-lifecycle).

### Try it in an isolated Pidgin profile

For an isolated user-level test, install the plugin and backend into a separate
Pidgin configuration directory:

```sh
SIGNAL_PURPLE_TEST_PROFILE="$HOME/.local/state/signal-purple-pidgin"

cmake -S . -B build-user -G Ninja -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_INSTALL_PREFIX="$SIGNAL_PURPLE_TEST_PROFILE" \
  -DSIGNAL_PURPLE_PLUGIN_DIR="$SIGNAL_PURPLE_TEST_PROFILE/plugins" \
  -DSIGNAL_CORE_INSTALL_DIR="$SIGNAL_PURPLE_TEST_PROFILE/plugins/signal-purple"
cmake --build build-user
cmake --install build-user

pidgin --config="$SIGNAL_PURPLE_TEST_PROFILE" --multiple --nologin
```

This keeps the test account, encrypted store, buddy list, and plugin copies
away from the normal `~/.purple` profile. The database secret still uses the
desktop's shared secret service; follow the
[account-removal instructions](#remove-account-data-and-the-linked-device) when
cleaning up a test link. A rootless installation into the normal profile can
instead use `$HOME/.purple` as the install prefix,
`$HOME/.purple/plugins` as the plugin directory, and
`$HOME/.purple/plugins/signal-purple` as the backend directory, but that is
not an isolated test.

## Installation lifecycle

### Upgrade

1. Disable the Signal account and close Pidgin so the encrypted database is
   quiescent.
2. Keep a copy of the database path shown in the account's advanced settings.
   The matching secret-service entry is labelled `signal-purple database for
   <account>` and is required to open that copy.
3. Install the complete new package or follow the source-install replacement
   procedure below. Never mix a plugin from one revision with a backend library
   from another.
4. Start Pidgin, enable the account, and confirm it reconnects without a QR,
   then confirm contacts, groups, and a direct send/receive round trip.

Store migrations are automatic and additive. Keep the pre-upgrade database and
secret until the new version has completed its validation period.

For a CMake source install, replace revisions instead of installing one over
the other. Inspect the `install_manifest.txt` saved from the currently
installed build, verify that every entry belongs to the active installation
prefix, and remove exactly those files. Then run `cmake --install` from the
complete target build. CMake can otherwise report a target artifact as
`Up-to-date` when the installed file has the same or a newer timestamp,
potentially leaving the plugin and backend on different revisions. Do not
remove a plugin directory recursively.

### Rollback

Close Pidgin and reinstall the previous complete package. For a CMake source
rollback, first remove exactly the files in the currently installed build's
saved manifest as described above, then install the complete previous build.
Do not run an older `cmake --install` over newer files or replace one shared
library independently. Restore the matching pre-upgrade database only if the
older version cannot open the upgraded copy. If a rollback cannot reconnect,
return to the new build and its database rather than deleting state. Relinking
is the last recovery option because it creates a new Signal linked device.

### Relink

To relink without destroying recoverable state, disable the account and choose
a new empty encrypted-store path in its advanced settings. Re-enable it and
scan the new QR. Remove the old linked device from an official Signal client
only after the replacement works.

### Remove installed files

Fully quit Pidgin before removing either library. A Debian package can be
removed with the package manager. A CMake source install has no automated
uninstall target: preserve the manifest when installing each revision and use
the manifest from the currently installed build. Verify its prefix, then remove
exactly the files it lists. Do not remove a plugin directory recursively, and
do not use a manifest from another prefix or revision. Remove both
`libsignal-purple.so` and its private
`signal-purple/libsignal_core.so` from the same installation scope.

Removing installed files leaves per-user account data intact.

### Remove account data and the linked device

Complete account removal is separate and irreversible. First disable and remove
the Purple account. Delete its database under `~/.purple/signal-purple/` or the
configured custom path, delete the matching labelled item from the desktop
secret service, and remove the linked device from an official Signal client.
Never delete only the database or only its secret if recovery may still be
needed.

## Link an account

1. In Pidgin, choose **Accounts → Manage Accounts → Add**.
2. Select **Signal** and enter a unique local account label. Purple does not
   store a Signal password.
3. Enable the account. A new encrypted store starts the QR linking flow.
4. On the primary phone, open **Signal Settings → Linked devices → Link new
   device** and scan the QR code.
5. Wait for pending Signal updates to finish. The plugin allows sends only
   after the linked-device queue is ready.

The encrypted SQLCipher database is stored under the Purple configuration
directory, normally
`~/.purple/signal-purple/<account-hash>.db3`. Its randomly generated
passphrase is stored in the desktop secret service through libsecret. Losing
either the database or its matching secret requires a new link. Treat the QR
code and any provisioning URI as sensitive credentials.

## How Signal maps into Pidgin

### Contacts and conversations

- Synchronized contacts appear in Pidgin's normal localized buddy group, such
  as **Friends** or **Buddies**. Signal does not expose presence, so they are
  shown as reachable while the account is connected.
- Signal group chats appear in Pidgin's normal **Chats** group. Local aliases,
  merged contacts, and custom placement are preserved during synchronization.
- Snapshot-confirmed contacts in the legacy **Signal** group and plugin-managed
  chats in **Signal groups** are moved to the normal groups; unrelated
  user-created entries are left alone.
- Direct and group messages are plain text. Incoming markup is escaped and
  outgoing Purple markup is stripped. Reactions become a short text message,
  and edits appear as new messages instead of changing earlier text. Outgoing
  message text is limited to 64 KiB.
- On reconnect, the plugin receives envelopes which Signal still has queued
  for this linked device. It cannot fetch arbitrary history or import an
  official client's local message database.

### Group actions

The plugin discovers current Signal groups; opening a saved Pidgin chat is a
local UI action, not a Signal join. Group sends and management stay unavailable
until an authoritative group refresh succeeds, and a failed refresh is retried
within the session. To leave remotely, right-click an active managed chat,
choose **Leave Signal group…**, and confirm. The chat is removed only after
Signal confirms the leave.

Pidgin's built-in **Remove Chat** and closing a conversation tab are local-only
because Purple 2 provides no protocol callback for those actions. A locally
removed chat can return after synchronization. Creating groups, inviting or
removing members, changing roles, titles, avatars, links, or join requests is
not implemented.

### Attachments

Valid incoming and within-limit outgoing files use Purple's transfer UI.
Incoming direct and group JPEG, PNG, and genuine GIF images render inline only
when their MIME type, signature, complete decode, 8 MiB encoded-size limit, and
bounded dimensions agree. Animated GIFs also have a cumulative decoded-frame
limit. Incoming direct and group MP4s which carry Signal's GIF flag are
converted through bounded, process-isolated FFmpeg pipes and presented as GIFs
when the optional `ffmpeg` and `prlimit` helpers are installed. Ordinary video
and any failed, unavailable, oversized, or invalid conversion retains the
original MP4 transfer prompt because Purple 2 has no native inline-video API.
Unnamed common media receives a usable type-specific filename. Empty or failed
incoming downloads are rejected visibly. Incoming transfer size follows
Signal's network policy without a lower plugin per-file or per-message cap.
Outgoing files are limited to 25 MiB. At most two outgoing files totaling 50 MiB
are admitted per account; retry after another transfer finishes or is cancelled
when that queue is full.
The [attachment policy](docs/attachment-policy.md) records the ownership and
memory bounds.

The plugin does not create a plaintext attachment cache. Inline images and
unresolved receive prompts remain in memory; an accepted transfer is written as
plaintext to the destination selected by the user. Outgoing file uploads are
cancellable but, unlike text messages, are not retained for automatic retry
after a failed upload or restart.

## Important limitations

- This is a secondary linked device only. Primary registration, account
  recovery, phone-number discovery, and remote contact editing are absent.
- Calls, stories, payments, backups, history import, disappearing-message
  timers, remote deletion, and view-once enforcement are absent. View-once
  media may therefore be exposed as a normal savable attachment.
- There is no in-plugin numeric safety-number comparison. Verify contacts with
  an official client or another trusted channel before accepting a changed
  identity.
- Signal-specific rich content is reduced or omitted; quotes, mentions,
  stickers, reactions, edits, and similar features do not have full native
  Purple representations.
- Typing and receipt state is best-effort and process-local. A restart can lose
  pending state, and asynchronous read-receipt failures are reported rather
  than durably replayed.
- Remote leave is the only implemented group-administration action. Other
  group administration remains unavailable.
- Presage currently materializes the durable replay result set in one database
  query, so peak startup memory scales with a very large offline backlog. The
  plugin limits in-flight projections, deferred live messages, and
  event/receipt queues, but linked devices with unusually large backlogs may
  still need extra memory.

## Security and local data

Protocol state, identity/session keys, group secrets, the durable text outbox,
and replay state are stored in SQLCipher. The database passphrase lives in the
desktop secret service, and the plugin refuses to fall back to plaintext when
that service is unavailable. The Rust FFI copies it directly into zeroizing
ownership, and the worker wipes that copy immediately after the SQLCipher store
open completes or is interrupted.

Identity handling uses trust on first use. An identity replacement for an
unverified contact produces a one-time warning and sending continues. A
replacement for an explicitly verified contact blocks sending until the user
accepts it after verification through another trusted channel.

Pidgin is a legacy in-process client: the UI and other loaded plugins can access
message memory. Direct and group conversations follow Purple's standard
user-controlled logging policy, which may store message transcripts outside
SQLCipher. In Pidgin, review Preferences > Logging and each conversation's
Options > Enable Logging. Disabling future logging does not delete existing
transcript files. Purple also stores aliases, group titles, canonical contact
identifiers, and opaque group identifiers in plaintext in `blist.xml`; a UI or
another plugin may retain more. No independent security audit has occurred.
Read the [security model](docs/security-model.md) before using a real account.

## Troubleshooting and bug reports

Use an isolated profile when collecting debug output so unrelated Purple
accounts are not loaded:

```sh
pidgin --config="$HOME/.local/state/signal-purple-pidgin" \
  --multiple --nologin --debug
```

Debug output can still contain private metadata. Never publish raw logs,
message text, phone numbers, service identifiers, QR provisioning data, keys,
or database secrets. Follow the [troubleshooting guide](docs/troubleshooting.md)
and include the plugin revision, distribution and version, Pidgin/libpurple
versions, build command, and sanitized output in a report.

Use [GitHub issues](https://github.com/adrighem/signal-purple/issues) for
reproducible bugs and feature proposals, [SUPPORT.md](SUPPORT.md) for support
guidance, and [SECURITY.md](SECURITY.md) for vulnerabilities.

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

Purple calls stay on the GLib main thread. Each account owns one Rust backend
actor, and a private nonblocking descriptor wakes the main context when events
are ready. See [architecture.md](docs/architecture.md) for lifecycle, storage,
message, contact, and group details.

## Project documentation

- [Changelog](CHANGELOG.md) and [roadmap](ROADMAP.md)
- [Compatibility policy](docs/compatibility.md) and
  [live-validation record](docs/live-validation.md)
- [Architecture](docs/architecture.md) and
  [security model](docs/security-model.md)
- [Development guide](docs/development.md),
  [dependency policy](docs/dependency-policy.md), and
  [Debian packaging](docs/debian-packaging.md)
- [Contributing](CONTRIBUTING.md), [support](SUPPORT.md), and
  [licensing details](docs/licensing.md)

## Acknowledgements

- [tdlib-purple](https://github.com/ars3niy/tdlib-purple) provided a practical
  precedent for adapting a modern messaging stack to libpurple and Pidgin.
- [Flare](https://gitlab.com/schmiddi-on-mobile/flare) provided an architectural
  reference for building a Signal client around Presage.

`signal-purple` is an independent implementation; these acknowledgements do
not imply copied code, endorsement, or affiliation.

## License and trademarks

Original C plugin code and general project material are licensed under
GPL-3.0-or-later. The Rust backend is AGPL-3.0-only because Presage and its
Signal stack use that license; AGPL terms apply to the combined binaries. See
[THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md) for exact pinned sources.

Signal, the Signal logo, and related names are trademarks of Signal Technology
Foundation. This project uses its own icon and is not endorsed by Signal.
