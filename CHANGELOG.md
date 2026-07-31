# Changelog

All notable changes will be documented here. The project follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and intends to use
semantic versioning after the first stable release.

## [0.4.2](https://github.com/adrighem/signal-purple/compare/v0.4.1...v0.4.2) (2026-07-31)


### Bug Fixes

* **sync:** keep receive stream independently polled ([f7b2d1e](https://github.com/adrighem/signal-purple/commit/f7b2d1e21d1bdcb82d4fc134952141e05012b29e))

## [0.4.1](https://github.com/adrighem/signal-purple/compare/v0.4.0...v0.4.1) (2026-07-31)


### Bug Fixes

* **sync:** defer contact request until queue drain ([abf0b06](https://github.com/adrighem/signal-purple/commit/abf0b06dcb215f0f0b0131ef1cb267b26f1d0761))

## [0.4.0](https://github.com/adrighem/signal-purple/compare/v0.3.1...v0.4.0) (2026-07-31)


### Features

* **media:** render Signal GIF videos inline ([0db5162](https://github.com/adrighem/signal-purple/commit/0db51620d45f63bc6593699c81b8e32a3d913cfa))

## [0.3.1](https://github.com/adrighem/signal-purple/compare/v0.3.0...v0.3.1) (2026-07-31)


### Bug Fixes

* **core:** bound downloads and preserve contact names ([d398240](https://github.com/adrighem/signal-purple/commit/d39824089ee4d06dae8cb58471aef101b3e0444a))
* **media:** present bounded GIFs inline ([5d390fe](https://github.com/adrighem/signal-purple/commit/5d390fec1ab7655cf5d9d14777afa32cf7837e13))

## [0.3.0](https://github.com/adrighem/signal-purple/compare/v0.2.4...v0.3.0) (2026-07-26)


### Features

* **purple:** honor standard conversation logging ([c596247](https://github.com/adrighem/signal-purple/commit/c5962472ee9f94508c859e043af7e87c6ce0de53))


### Bug Fixes

* **core:** bound backend shutdown phases ([ce0de33](https://github.com/adrighem/signal-purple/commit/ce0de3343e2792a4132158b8e341bed4394bd29d))
* **core:** make control paths reliable ([374a92a](https://github.com/adrighem/signal-purple/commit/374a92a8bc099a0bd9d6b91569fc759ef374ab57))
* **core:** minimize passphrase lifetime ([a9760d1](https://github.com/adrighem/signal-purple/commit/a9760d12868d46723595779679c5d03adc23562b))
* **core:** serialize message timestamp allocation ([2dbae8c](https://github.com/adrighem/signal-purple/commit/2dbae8c851c2213466b159a273953063aed51fa9))
* **core:** serialize SQLite store access ([6834a5c](https://github.com/adrighem/signal-purple/commit/6834a5c536e293d969ea7968d7c1f52c3d82251b))
* harden final teardown boundaries ([45171d0](https://github.com/adrighem/signal-purple/commit/45171d04a3d0c285784f928661cc9c03d6719fa0))
* **plugin:** harden outgoing attachment lifecycle ([ccb548f](https://github.com/adrighem/signal-purple/commit/ccb548fc204c68d6a1022efe1c6146c92751507d))

## [0.2.4](https://github.com/adrighem/signal-purple/compare/v0.2.3...v0.2.4) (2026-07-25)


### Bug Fixes

* bound outgoing attachment admission ([a0825d4](https://github.com/adrighem/signal-purple/commit/a0825d419f5bd006009789c6757248efdcdba226))

## [0.2.3](https://github.com/adrighem/signal-purple/compare/v0.2.2...v0.2.3) (2026-07-25)


### Bug Fixes

* preserve release version marker ([afee503](https://github.com/adrighem/signal-purple/commit/afee503b3b06c0494bbc941e0cfab38b7ad6c6ce)), closes [#21](https://github.com/adrighem/signal-purple/issues/21)
* preserve release version marker ([2c3aaef](https://github.com/adrighem/signal-purple/commit/2c3aaefff3156db3fb2d49a6d29a7920222847bd)), closes [#20](https://github.com/adrighem/signal-purple/issues/20)
* preserve release version marker ([d5dc805](https://github.com/adrighem/signal-purple/commit/d5dc80503c7f9aa4397e0e7085806dfafe2797c3)), closes [#18](https://github.com/adrighem/signal-purple/issues/18)

## 0.2.2 (2026-07-24)

### Changed

- Declare signal-purple alpha quality while retaining its unofficial status,
  narrow supported environment, and explicit compatibility limitations.

### Fixed

- Recover linked Signal connections after transient network failures while
  preserving queued group operations.
- Resolve the local account in group participant lists by its canonical Signal
  identifier, preferring the local Purple account alias and falling back to the
  account's remote Signal profile name.

## 0.2.1 (2026-07-22)

### Fixed

- Show accepted outgoing group messages in the active Purple chat exactly once
  without treating them as received messages.

## 0.2.0 (2026-07-21)

Version 0.2.0 is the first version intended for a tagged public pre-release.
The internal 0.1.0 bootstrap was not tagged or published, so these notes cover
the project to date.

### Fixed

- Keep replayed group messages in the saved Pidgin chat during reconnect by
  restoring its local display title after membership refreshes.
- Treat locally sent messages as outgoing during crash recovery and mark their
  stored copies as projected after successful sends, avoiding misrouted or
  duplicated conversations on reconnect.
- Render decoder-validated, dimension-bounded incoming JPEG and PNG images
  inside their originating group conversation instead of presenting the MIME
  type and a direct file transfer from the sender.
- Adopt contacts from the exact legacy `Signal` group only when the current
  authoritative snapshot confirms the same account and identifier, allowing
  older profiles to migrate without moving custom or unrelated buddies.

### Changed

- Place synchronized contacts and chats in Purple's localized default groups,
  migrating plugin-managed nodes from the former Signal-specific groups while
  preserving custom placement.
- Use each group's stable opaque identifier as its Purple conversation identity
  while preserving user-set local chat aliases across Signal title refreshes.
- Replace the fixed 20 ms backend polling timer with descriptor-driven GLib
  wakeups, eliminating roughly 50 idle main-loop wakeups per second per account.
- Fully refresh Storage Service and cached group candidates before publishing
  an authoritative snapshot, pruning only groups confirmed inaccessible or no
  longer containing this account. Group joins and sends are restricted to the
  active set.

### Added

- Add a confirmed **Leave Signal group…** chat action which performs a remote
  Signal leave and removes the managed Purple chat only after success.
- Purple 2.14 protocol plugin with direct and group text-message routing.
- Pinned Presage Rust backend with linked-device QR provisioning.
- SQLCipher state protected by a libsecret-managed passphrase.
- Automatic contact refresh plus authoritative buddy-list
  create/update/delete, group metadata sync, typing indicators, and delivery
  receipts.
- Versioned polling C ABI with owned event memory and deterministic teardown.
- C utility tests, Rust unit tests, and a headless libpurple plugin probe.
- Documentation of queued offline-message delivery, contact-sync diagnostics,
  and the corresponding Flare design.
- Durable frontend message replay, encrypted text-message retry, read receipts,
  and identity-change acceptance without relinking.
- Bounded direct and group attachment transfers through Purple's native file
  transfer UI, including cancellable uploads and sanitized incoming filenames.

## 0.1.0 (2026-07-19)

Internal versioning bootstrap. This version was not tagged or published; its
user-facing changes are included in the 0.2.0 notes above.
