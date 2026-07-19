# Maintenance runs

## 2026-07-19

- Inbox: no unread notifications.
- Open issues: none.
- Open pull requests: PR #1 was assessed as low risk and merged.
- Dependabot alerts: none.
- Code scanning alerts: none; the initial CodeQL default-setup run passed for
  Actions, C/C++, and Rust.
- Repository safeguards: GitHub Actions are enabled with read-only default
  workflow permissions. Branch protection was deliberately not added because
  this is currently a single-developer repository.
- Artifact: `notes/pr-1.md`.
- Shipped locally: ABI v2 contact snapshot boundaries and authoritative Purple
  buddy-list create/update/delete, with focused reconciliation tests.
- System install: release build installed under `/usr/lib/x86_64-linux-gnu/purple-2`;
  the installed plugin passed the headless probe.
- Isolated-profile validation passed fresh QR linking, encrypted-store
  reconnect without a new QR, and direct messages in both directions.
- The first unsolicited snapshot contained one contact. An explicit contact
  request synchronized 46 contacts and projected all 46 as visible Purple
  buddies. Alias-update and stale-delete behavior remains test-only. Groups,
  typing, receipts, and second-device sync remain pending.
- User documentation now distinguishes queued offline delivery from unsupported
  historical import and records the Flare contact-sync comparison.

The installed maintainer package did not include its documented triage script
or reference files, so this run was captured manually and `state.json` was left
unchanged rather than guessing its schema.
