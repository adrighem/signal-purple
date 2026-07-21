# Changelog

All notable changes will be documented here. The project follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and intends to use
semantic versioning after the first stable release.

## [0.2.0](https://github.com/adrighem/signal-purple/compare/v0.1.0...v0.2.0) (2026-07-21)


### Features

* add bounded attachment transfers ([9f68633](https://github.com/adrighem/signal-purple/commit/9f68633a6f4d577e65c4bd34e88e7a38a08d5cec))
* complete basic Signal group support ([06fe4d2](https://github.com/adrighem/signal-purple/commit/06fe4d2f684416ce8094a3eb1f1e782893da1ccb))
* handle Signal identity replacements ([a57f0fe](https://github.com/adrighem/signal-purple/commit/a57f0fec53664f0db09b21002caf28688a29cdee))
* replay messages until Purple acknowledges them ([1c252e6](https://github.com/adrighem/signal-purple/commit/1c252e64632d2550fec6e60df1b7129171f9d457))
* retry sends from an encrypted outbox ([adc0661](https://github.com/adrighem/signal-purple/commit/adc06610beab8f698485cc2227b4a876e4b4ad3d))
* send read receipts when conversations are focused ([7b42ba2](https://github.com/adrighem/signal-purple/commit/7b42ba2f8c0ef6286f64f0b43e299d52f9ea4d62))
* synchronize Signal contacts with Purple ([7fb1269](https://github.com/adrighem/signal-purple/commit/7fb1269eefe0ca2f940e2f6ec9aaf2c58903b8da))
* synchronize Signal groups with Purple ([7a1a752](https://github.com/adrighem/signal-purple/commit/7a1a752491914e758993c5462ae121edcfbfcff6))


### Bug Fixes

* adopt legacy Signal contacts ([6e81640](https://github.com/adrighem/signal-purple/commit/6e81640b11d52235af1e7a88395706442e6282b6))
* avoid logging Signal identifiers ([ee9f8f6](https://github.com/adrighem/signal-purple/commit/ee9f8f66ff57252dcf3cca4b326914d6f31f7493))
* preserve group routing across reconnects ([822414a](https://github.com/adrighem/signal-purple/commit/822414aa8a3b91ad0d07cc336e318d6f7e8b3a20))
* render group images inline ([c0c9d91](https://github.com/adrighem/signal-purple/commit/c0c9d91bcc5c96978d59e863f1f5c51a44df9a10))
* render incoming text as chat messages ([79e554c](https://github.com/adrighem/signal-purple/commit/79e554c8249cffac56e7c0dbd51aa03569a8b8d9))


### Reverts

* restore main after misdirected hardening push ([1bd2c90](https://github.com/adrighem/signal-purple/commit/1bd2c907943a97a55b53a4c9ec1e9eb7dd67ff33)), closes [#5](https://github.com/adrighem/signal-purple/issues/5)

## 0.1.0 (2026-07-19)


### Features

* add bounded attachment transfers ([9f68633](https://github.com/adrighem/signal-purple/commit/9f68633a6f4d577e65c4bd34e88e7a38a08d5cec))
* handle Signal identity replacements ([a57f0fe](https://github.com/adrighem/signal-purple/commit/a57f0fec53664f0db09b21002caf28688a29cdee))
* replay messages until Purple acknowledges them ([1c252e6](https://github.com/adrighem/signal-purple/commit/1c252e64632d2550fec6e60df1b7129171f9d457))
* retry sends from an encrypted outbox ([adc0661](https://github.com/adrighem/signal-purple/commit/adc06610beab8f698485cc2227b4a876e4b4ad3d))
* send read receipts when conversations are focused ([7b42ba2](https://github.com/adrighem/signal-purple/commit/7b42ba2f8c0ef6286f64f0b43e299d52f9ea4d62))
* synchronize Signal contacts with Purple ([7fb1269](https://github.com/adrighem/signal-purple/commit/7fb1269eefe0ca2f940e2f6ec9aaf2c58903b8da))
* synchronize Signal groups with Purple ([7a1a752](https://github.com/adrighem/signal-purple/commit/7a1a752491914e758993c5462ae121edcfbfcff6))


### Bug Fixes

* avoid logging Signal identifiers ([ee9f8f6](https://github.com/adrighem/signal-purple/commit/ee9f8f66ff57252dcf3cca4b326914d6f31f7493))
* render incoming text as chat messages ([79e554c](https://github.com/adrighem/signal-purple/commit/79e554c8249cffac56e7c0dbd51aa03569a8b8d9))

## [Unreleased]

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
