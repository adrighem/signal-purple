# Decisions

## 2026-07-18 — initial architecture

- Target Purple 2.14 on Debian 13.
- Use an owned polling C ABI and a Presage-backed Rust actor.
- Link as a secondary device; defer primary registration.
- Encrypt Presage storage with SQLCipher and hold its passphrase in libsecret.
- License original C/project work GPL-3.0-or-later and the Rust backend
  AGPL-3.0-only; document combined-distribution obligations.
