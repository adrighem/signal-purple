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
- The backend uses Presage's `OnNewIdentity::Reject` policy.
- Group keys and string lengths are validated before backend use. Group master
  keys are never exposed as Purple chat names or join metadata.
- Remote message text is escaped before Purple renders it.
- Conversation logging is disabled and each delivered message carries Purple's
  no-log flag.
- Upstream info-level tracing is compiled out so Presage cannot emit the
  provisioning URI through its linking log statement.
- Worker shutdown is joined before account state is freed.
- Backend events use a bounded queue; overflow fails visibly and reconnects
  rather than allowing unbounded process memory growth.

## Known gaps

- No independent security audit has occurred.
- Live direct-message, startup-backlog, and contact-sync paths have been
  verified. The full supported-client and failure-mode matrix remains a release
  gate.
- Identity-key rejection is not paired with a full safety-number review and
  approval UI. A changed contact remains blocked in the current store; official
  client verification does not update this linked device's Presage database.
  Pinned Presage may skip inbound identity/decryption failures before yielding
  an event, so Purple cannot reliably warn about that skipped message.
- Attachments are names-only; file transfer security and encrypted caching are
  not implemented.
- Pidgin/libpurple 2 is a legacy in-process plugin environment. A memory-safety
  flaw in the UI or another plugin can access this process.
- Signal does not support third-party clients or promise protocol stability.
- Disappearing timers and remote deletions are not projected into Purple.
- Purple's buddy list stores synced aliases and stable Signal identifiers in
  plaintext. The plugin cannot prevent another in-process UI or plugin from
  retaining message text despite its no-log defaults.

## Update response

Security and compatibility dependency updates require a full diff/provenance
review, Rust/C checks, store migration tests, and live non-production account
tests before release. Do not merge automated Signal-stack bumps solely because
they compile.
