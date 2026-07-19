# Changelog

All notable changes will be documented here. The project follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and intends to use
semantic versioning after the first stable release.

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

### Added

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
