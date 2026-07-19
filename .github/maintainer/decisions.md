# Decisions

## 2026-07-18 — initial architecture

- Target Purple 2.14 on Debian 13.
- Use an owned polling C ABI and a Presage-backed Rust actor.
- Link as a secondary device; defer primary registration.
- Encrypt Presage storage with SQLCipher and hold its passphrase in libsecret.
- License original C/project work GPL-3.0-or-later and the Rust backend
  AGPL-3.0-only; document combined-distribution obligations.

## 2026-07-19 - contact buddy-list ownership

- Request a fresh contact sync from the primary device on every connection;
  an unsolicited initial queue is not guaranteed to contain the full address
  book.
- Treat a successfully decoded Signal contact snapshot as authoritative for
  plugin-managed Purple buddies.
- Preserve user-created, unmanaged buddies and local aliases.
- Apply deletions only after an explicit completed snapshot boundary; a store
  read or decode error must leave the existing buddy list intact.
- Treat synchronized contacts as reachable while connected because Signal has
  no presence API and Purple otherwise hides the address book by default.
- Keep Signal contact mutation and phone-number discovery out of scope.
