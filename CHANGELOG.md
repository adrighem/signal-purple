# Changelog

All notable changes will be documented here. The project follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and intends to use
semantic versioning after the first stable release.

## [1.2.0](https://github.com/adrighem/signal-purple/compare/v1.1.0...v1.2.0) (2026-08-06)


### Features

* add Ubuntu release packages ([#52](https://github.com/adrighem/signal-purple/issues/52)) ([7d3b9f7](https://github.com/adrighem/signal-purple/commit/7d3b9f716008ffd64216041f1af159490cd03753))
* **release:** integrate Nix flake and Fedora RPM package generation ([75e1234](https://github.com/adrighem/signal-purple/commit/75e1234d70acd85660b8a029f0e18f7997df4146))
* **release:** integrate Nix flake and Fedora RPM package generation ([38eef86](https://github.com/adrighem/signal-purple/commit/38eef86d55fe487c5bfc1e6dce7222a71029c503))


### Bug Fixes

* keep receipt tasks with projection state ([6d3119a](https://github.com/adrighem/signal-purple/commit/6d3119a48d1fbfbbac3a3e89da52e497f480507e))
* retry receipts after websocket recovery ([fdf0ae4](https://github.com/adrighem/signal-purple/commit/fdf0ae422e084e3ba30018a5d6c66b08bb1b5d62))
* retry receipts after websocket recovery ([74e34dd](https://github.com/adrighem/signal-purple/commit/74e34dd2d94ae4b600ced71f4836d7797e76929f))

## [1.1.0](https://github.com/adrighem/signal-purple/compare/v1.0.0...v1.1.0) (2026-08-01)


### Features

* add bounded attachment transfers ([9f68633](https://github.com/adrighem/signal-purple/commit/9f68633a6f4d577e65c4bd34e88e7a38a08d5cec))
* complete basic Signal group support ([06fe4d2](https://github.com/adrighem/signal-purple/commit/06fe4d2f684416ce8094a3eb1f1e782893da1ccb))
* handle Signal identity replacements ([a57f0fe](https://github.com/adrighem/signal-purple/commit/a57f0fec53664f0db09b21002caf28688a29cdee))
* **media:** render Signal GIF videos inline ([0db5162](https://github.com/adrighem/signal-purple/commit/0db51620d45f63bc6593699c81b8e32a3d913cfa))
* **purple:** honor standard conversation logging ([c596247](https://github.com/adrighem/signal-purple/commit/c5962472ee9f94508c859e043af7e87c6ce0de53))
* replay messages until Purple acknowledges them ([1c252e6](https://github.com/adrighem/signal-purple/commit/1c252e64632d2550fec6e60df1b7129171f9d457))
* retry sends from an encrypted outbox ([adc0661](https://github.com/adrighem/signal-purple/commit/adc06610beab8f698485cc2227b4a876e4b4ad3d))
* send read receipts when conversations are focused ([7b42ba2](https://github.com/adrighem/signal-purple/commit/7b42ba2f8c0ef6286f64f0b43e299d52f9ea4d62))
* synchronize Signal contacts with Purple ([7fb1269](https://github.com/adrighem/signal-purple/commit/7fb1269eefe0ca2f940e2f6ec9aaf2c58903b8da))
* synchronize Signal groups with Purple ([7a1a752](https://github.com/adrighem/signal-purple/commit/7a1a752491914e758993c5462ae121edcfbfcff6))


### Bug Fixes

* adopt legacy Signal contacts ([6e81640](https://github.com/adrighem/signal-purple/commit/6e81640b11d52235af1e7a88395706442e6282b6))
* avoid logging Signal identifiers ([ee9f8f6](https://github.com/adrighem/signal-purple/commit/ee9f8f66ff57252dcf3cca4b326914d6f31f7493))
* bound outgoing attachment admission ([a0825d4](https://github.com/adrighem/signal-purple/commit/a0825d419f5bd006009789c6757248efdcdba226))
* **build:** prevent plugin symbol interposition ([4bc1e4a](https://github.com/adrighem/signal-purple/commit/4bc1e4a3cb066e4f53d8c13b0e18e65dbc9e9217))
* **core:** bound backend shutdown phases ([ce0de33](https://github.com/adrighem/signal-purple/commit/ce0de3343e2792a4132158b8e341bed4394bd29d))
* **core:** bound downloads and preserve contact names ([d398240](https://github.com/adrighem/signal-purple/commit/d39824089ee4d06dae8cb58471aef101b3e0444a))
* **core:** make control paths reliable ([374a92a](https://github.com/adrighem/signal-purple/commit/374a92a8bc099a0bd9d6b91569fc759ef374ab57))
* **core:** minimize passphrase lifetime ([a9760d1](https://github.com/adrighem/signal-purple/commit/a9760d12868d46723595779679c5d03adc23562b))
* **core:** serialize message timestamp allocation ([2dbae8c](https://github.com/adrighem/signal-purple/commit/2dbae8c851c2213466b159a273953063aed51fa9))
* **core:** serialize SQLite store access ([6834a5c](https://github.com/adrighem/signal-purple/commit/6834a5c536e293d969ea7968d7c1f52c3d82251b))
* echo outgoing group messages ([23bae50](https://github.com/adrighem/signal-purple/commit/23bae50d48f5004f2fecaa6bb3c7a7519da36193))
* echo outgoing group messages ([1b05316](https://github.com/adrighem/signal-purple/commit/1b0531614c546ce137e5adb6008b75cc5ff9f204))
* harden final teardown boundaries ([45171d0](https://github.com/adrighem/signal-purple/commit/45171d04a3d0c285784f928661cc9c03d6719fa0))
* **media:** present bounded GIFs inline ([5d390fe](https://github.com/adrighem/signal-purple/commit/5d390fec1ab7655cf5d9d14777afa32cf7837e13))
* normalize the release SBOM root identity ([1298f42](https://github.com/adrighem/signal-purple/commit/1298f42e6ffb4c4e631beb5089b6a20b5dc09597))
* **plugin:** harden outgoing attachment lifecycle ([ccb548f](https://github.com/adrighem/signal-purple/commit/ccb548fc204c68d6a1022efe1c6146c92751507d))
* preserve group routing across reconnects ([822414a](https://github.com/adrighem/signal-purple/commit/822414aa8a3b91ad0d07cc336e318d6f7e8b3a20))
* preserve release version marker ([afee503](https://github.com/adrighem/signal-purple/commit/afee503b3b06c0494bbc941e0cfab38b7ad6c6ce)), closes [#21](https://github.com/adrighem/signal-purple/issues/21)
* preserve release version marker ([2c3aaef](https://github.com/adrighem/signal-purple/commit/2c3aaefff3156db3fb2d49a6d29a7920222847bd)), closes [#20](https://github.com/adrighem/signal-purple/issues/20)
* preserve release version marker ([d5dc805](https://github.com/adrighem/signal-purple/commit/d5dc80503c7f9aa4397e0e7085806dfafe2797c3)), closes [#18](https://github.com/adrighem/signal-purple/issues/18)
* **projection:** retain rejected events for replay ([2de2563](https://github.com/adrighem/signal-purple/commit/2de2563f85e655cabb53181d150c9f258a8d0781))
* recover Signal connections after network loss ([3a213d5](https://github.com/adrighem/signal-purple/commit/3a213d5d3397edded47790e78b26d9c38a9b08d5))
* recover Signal connections after network loss ([77100c9](https://github.com/adrighem/signal-purple/commit/77100c9bcc6a5e627d4ba1607185d911a6d7be9f))
* render group images inline ([c0c9d91](https://github.com/adrighem/signal-purple/commit/c0c9d91bcc5c96978d59e863f1f5c51a44df9a10))
* render incoming text as chat messages ([79e554c](https://github.com/adrighem/signal-purple/commit/79e554c8249cffac56e7c0dbd51aa03569a8b8d9))
* resolve draft releases during artifact upload ([34e9b67](https://github.com/adrighem/signal-purple/commit/34e9b672442982f8e393756f33e73cb595031646))
* resolve the local group participant alias ([486ed2a](https://github.com/adrighem/signal-purple/commit/486ed2a36d6fd13b4c6f70431d05fa2f09a0dc1e))
* restore release-please version marker in Cargo.lock ([4d6f3da](https://github.com/adrighem/signal-purple/commit/4d6f3daf2a96bc6a42be071758b4f4ab20fe1785))
* support annotated source archive tags ([8c0ebe3](https://github.com/adrighem/signal-purple/commit/8c0ebe32de1f1aa55d489e2dadc24bf21fdce14c)), closes [#5](https://github.com/adrighem/signal-purple/issues/5)
* **sync:** defer contact request until queue drain ([abf0b06](https://github.com/adrighem/signal-purple/commit/abf0b06dcb215f0f0b0131ef1cb267b26f1d0761))
* **sync:** keep receive stream independently polled ([f7b2d1e](https://github.com/adrighem/signal-purple/commit/f7b2d1e21d1bdcb82d4fc134952141e05012b29e))


### Reverts

* restore main after misdirected hardening push ([1bd2c90](https://github.com/adrighem/signal-purple/commit/1bd2c907943a97a55b53a4c9ec1e9eb7dd67ff33)), closes [#5](https://github.com/adrighem/signal-purple/issues/5)

## [0.4.4](https://github.com/adrighem/signal-purple/compare/v0.4.3...v0.4.4) (2026-07-31)


### Bug Fixes

* **projection:** retain rejected events for replay ([2de2563](https://github.com/adrighem/signal-purple/commit/2de2563f85e655cabb53181d150c9f258a8d0781))

## [0.4.3](https://github.com/adrighem/signal-purple/compare/v0.4.2...v0.4.3) (2026-07-31)


### Bug Fixes

* **build:** prevent plugin symbol interposition ([4bc1e4a](https://github.com/adrighem/signal-purple/commit/4bc1e4a3cb066e4f53d8c13b0e18e65dbc9e9217))

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
