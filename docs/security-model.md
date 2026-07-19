# Security model

## Assets

The Rust store contains linked-device credentials, identity/session keys,
contacts, groups, and message state. Its randomly generated SQLCipher
passphrase is held by the user's secret service via libsecret, not in Purple's
account XML. The default data directory is restricted to mode `0700`.

QR provisioning URIs, passphrases, keys, canonical identifiers, phone numbers,
and message bodies are sensitive. Production code must not log them.

## Trust boundaries

- Purple/Pidgin UI and the C adapter share one process.
- The Rust backend is a separate shared library in that process and runs its
  protocol work on a dedicated thread.
- The versioned ABI copies commands and transfers owned event allocations. No
  Presage object or borrowed Rust pointer crosses it.
- Presage and its Signal dependencies are unreviewed upstream code from pinned
  revisions. Pinning improves reproducibility, not trustworthiness.

## Current protections

- SQLCipher is enabled by Presage's SQLite-store default feature.
- A missing libsecret service fails closed; the plugin does not fall back to an
  unencrypted database.
- Identity replacements are recorded in SQLCipher. Receiving continues so a
  service-acknowledged envelope is not silently lost. Unverified contacts also
  continue sending after one advisory. Replacements for explicitly verified
  contacts remain blocked for sending until the user accepts them from the
  buddy menu, at which point sessions are reset and the contact is downgraded
  to unverified.
- Group master keys remain in the encrypted Rust store. Purple persists only a
  domain-separated SHA-256 group identifier, which the backend resolves against
  the store for sends.
- Remote message text is escaped before Purple renders it.
- Conversation logging is disabled on every Signal conversation. Text messages
  use normal send/receive flags because Pidgin renders Purple's per-message
  no-log flag as an informational notice instead of a chat message.
- Upstream info-level tracing is compiled out so Presage cannot emit the
  provisioning URI through its linking log statement.
- Worker shutdown is joined before account state is freed.
- Backend events use a bounded queue; overflow fails visibly and reconnects
  rather than allowing unbounded process memory growth.
- Message projection state is stored in the same SQLCipher database. Purple
  acknowledges a message event only after its synchronous conversation write;
  unacknowledged content is replayed after the next receive queue drain.
- Read receipts are emitted only after Purple reports focus. Pending receipt
  metadata is held in process memory and is not written to Purple's plaintext
  configuration.
- Unsent message bodies, recipients, timestamps, and retry counters remain in
  the SQLCipher outbox. Purple receives errors at the first failure and at
  bounded later attempts without logging the message body.

## Known gaps

- No independent security audit has occurred.
- Live direct-message, contact-sync, group-discovery, and
  group-membership paths have been verified. The full supported-client and
  failure-mode matrix, including startup backlog exactly-once behavior, remains
  a release gate.
- Purple does not display or compare the numeric safety number. Acceptance is
  therefore a confirmation that the user completed verification through
  another trusted channel, not an in-plugin cryptographic comparison. This
  path is unit-tested but still needs a controlled live identity replacement.
- Attachments are names-only; file transfer security and encrypted caching are
  not implemented.
- Pidgin/libpurple 2 is a legacy in-process plugin environment. A memory-safety
  flaw in the UI or another plugin can access this process.
- Signal does not support third-party clients or promise protocol stability.
- Disappearing timers and remote deletions are not projected into Purple.
- Purple's buddy list stores synced contact aliases, group titles, canonical
  contact identifiers, and opaque group identifiers in plaintext. The plugin
  cannot prevent another in-process UI or plugin from retaining message text
  despite its no-log defaults.

## Update response

Security and compatibility dependency updates require a full diff/provenance
review, Rust/C checks, store migration tests, and live non-production account
tests before release. Do not merge automated Signal-stack bumps solely because
they compile.
