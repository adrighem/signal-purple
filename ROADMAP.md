# Roadmap

## 0.1 — prove interoperability

- Complete linked-device, reconnect, direct-message, group-message, typing,
  and delivery-receipt tests with dedicated non-production Signal accounts.
- Add deterministic reconnect and malformed-event regression coverage.
- Vendor the Rust dependency graph for network-free release builds.
- Produce a reproducible Debian 13 package.

## 0.2 — secure messaging UX

- Add an explicit upstream identity-change event, then implement safety-number
  display, key-change review, and explicit trust.
- Add read-receipt presentation without misleading per-device status.
- Improve aliases, usernames, profiles, reactions, edits, and deletion notices.

## 0.3 — attachments

- Add bounded attachment upload/download with Purple transfer objects.
- Encrypt cached content and define cleanup/recovery semantics.

## Deferred

Primary-device registration, active phone-number discovery, calls, official
backup compatibility, and advanced group administration remain deferred until
the linked-device messaging core is demonstrably reliable.
