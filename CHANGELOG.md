# Changelog

All notable changes will be documented here. The project follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and intends to use
semantic versioning after the first stable release.

## [Unreleased]

### Fixed

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
