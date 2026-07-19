# Changelog

All notable changes will be documented here. The project follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and intends to use
semantic versioning after the first stable release.

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
