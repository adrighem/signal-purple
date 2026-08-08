# Security model

## Assets

The Rust store contains linked-device credentials, identity/session keys,
contacts, groups, and message state. Its randomly generated SQLCipher
passphrase is held by the user's secret service via libsecret, not in Purple's
account XML. The FFI copy immediately enters zeroizing ownership and is wiped
after the SQLCipher open attempt, including error or shutdown paths. It is not
retained for the worker session. The default data directory is restricted to
mode `0700`.

QR provisioning URIs, passphrases, keys, canonical identifiers, phone numbers,
and message bodies are sensitive. signal-purple-owned diagnostic messages must
not deliberately interpolate them. Raw upstream error text remains a trust
boundary and can reach Purple diagnostics, so diagnostic output must still be
treated as sensitive and sanitized before sharing. User-controlled Purple
conversation transcripts are an explicit UI data feature and may contain
message bodies plus conversation or participant identifiers; they are
user-facing local storage, not diagnostic output.

## Trust boundaries

- Purple/Pidgin UI and the C adapter share one process.
- The Rust backend is a separate shared library in that process and runs its
  protocol work on a dedicated thread.
- The versioned ABI copies commands and transfers owned event allocations. No
  Presage object or ownerless borrowed Rust pointer crosses it. Event payload
  pointers borrow from their Rust-owned event allocation until
  `signal_event_free`.
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
  domain-separated SHA-256 group identifier. Before publishing membership, the
  backend verifies the exact Storage Service record-key set, refreshes every
  discovered or cached candidate from GroupsV2, and commits the result in one
  SQLite transaction. Group sends resolve only against that active set.
- Remote message text is escaped before Purple renders it.
- Direct and group conversations follow Purple's user-controlled logging
  policy. Libpurple 2 defaults new conversation logging on unless the profile
  changes it, and the frontend may apply a conversation-specific setting.
  These transcripts are owned by Purple and are not encrypted by
  signal-purple's SQLCipher store.
- Upstream info-level tracing is compiled out so Presage cannot emit the
  provisioning URI through its linking log statement.
- Worker shutdown cancels admitted attachments before signaling and joining the
  backend, and account state is freed only after that join. The backend races
  store registration and initialization, profile lookup, group synchronization,
  durable replay, outbox retry, and live projection waits against shutdown.
  Filesystem preparation completes before the worker is created, so it cannot
  outlive the core. Cleanup and Tokio runtime shutdown have bounded budgets.
  Dropping an async wait does not interrupt an already dispatched SQLx SQLite
  call, so the Linux backend is marked ELF `NODELETE` and remains mapped until
  process exit while dependency-owned work finishes. No late dependency work
  calls C or Purple. Interrupted projections and undrained acknowledgements
  remain eligible for replay, and interrupted outbox attempts retain their
  encrypted rows.
- Backend events use a bounded queue. Count and aggregate-byte pressure block
  the producer until Purple drains capacity; teardown wakes blocked producers.
  One event larger than the normal 64 MiB aggregate budget is admitted alone.
- Incoming attachment size follows Signal's network policy without a lower
  plugin per-file or per-message cap. Incoming data and unresolved Purple
  receive prompts remain in memory, including temporary handoff copies.
  Sender-declared plaintext size cannot drive download allocation; it is used
  only after authenticated decryption to remove Signal privacy padding.
  Outgoing attachments are capped at 25 MiB. The C adapter rejects non-regular
  and known-oversized outgoing files before allocating their contents, rejects
  empty files, and enforces the same limit while reading from the already
  inspected descriptor. Every outgoing Purple transfer is registered from
  creation. Start callbacks are protected by a
  temporary reference, started transfers are cancelled on disconnect, and all
  remaining contexts are detached before connection state is freed.
  Outgoing queued, recovery-deferred, and active attachments share a per-account
  limit of two files and 50 MiB. Admission capacity and active request identity
  are released together on every terminal path. Cancellation is stored on the
  admitted request rather than submitted through the bounded work queue, so it
  remains effective under queue pressure and before task startup. Binary
  backend events normally have a 64 MiB aggregate ceiling; see the
  [attachment policy](attachment-policy.md). Direct and group images are
  eligible for inline display only when a JPEG, PNG, or GIF MIME type agrees
  with its file signature, the encoded payload is no larger than 8 MiB, and the
  complete payload decodes. Dimensions are no larger than 8192 pixels per edge
  or 16 megapixels total. GIFs are additionally limited to 8 million cumulative
  canvas pixel-frames; their block structure is checked before decoder
  allocation. Decoder validation is chunked, and rejected dimensions are
  scaled down before allocation. A Signal GIF-flagged direct or group MP4 may be
  converted to a GIF by a pipe-only FFmpeg child with a cleared environment and
  fixed arguments. `prlimit` applies a 1 GiB address-space cap
  plus CPU and file-descriptor limits; the worker separately bounds
  source/output bytes, wall time, threads, dimensions, frame rate, cumulative
  frame area, attempts, and concurrency. Invalid output and all conversion
  failures preserve the original receive prompt. Ordinary video and other data
  also use that prompt. One incoming-presentation gate applies Purple's direct
  privacy and group-ignore rules to text, inline media, and transfer fallback.
  Filtered events are acknowledged without presentation or a read receipt;
  outgoing linked-device echoes remain visible.
  Decrypted
  incoming bytes remain in memory until the displayed image is released, saved,
  or rejected; the plugin creates no plaintext attachment cache. Remote
  filenames are reduced to a basename before Purple uses them.
- Message projection state is stored in the same SQLCipher database. Purple
  acknowledges a message event only after the adapter validates its required
  routing and payload fields and accepts its synchronous presentation or
  terminal policy rejection. Malformed or interrupted content remains
  unacknowledged and is replayed after the next receive queue drain. Accepted
  acknowledgements use a coalescing inbox bounded by pending projection IDs,
  retry local store failures, and drain on orderly shutdown.
- Read receipts are emitted only after Purple reports focus. Pending receipt
  metadata is deduplicated by exact recipient, group, and timestamp, capped at
  4096 entries, retried after synchronous queue pressure or backend readiness,
  and held only in process memory. Delivery-receipt metadata has the same 4096
  entry bound. Neither queue is restart-persistent, and read-receipt failures
  after backend admission are reported rather than durably replayed. Excess
  receipt metadata is discarded with a rate-limited warning rather than
  permitting unbounded memory growth.
- Unsent message bodies, recipients, timestamps, and retry counters remain in
  the SQLCipher outbox. Purple receives errors at the first failure and at
  bounded later attempts, and those error events omit the message body.
- Adapter-generated text, attachment, typing, and receipt sends share an atomic
  per-core timestamp allocator. Every new allocation remains strictly
  increasing under concurrent sends and wall-clock rollback. Durable retries
  reuse their original protocol timestamp, and retry scheduling uses a separate
  wall-clock value.

## Known gaps

- No independent security audit has occurred.
- The pinned Presage store API reads the full unprojected-message result set in
  one query. signal-purple performs that read once per connection and limits
  acknowledgement-pending projections and separately deferred live messages to
  64 each, but peak replay-query memory still scales with the durable backlog
  until Presage exposes cursor pagination.
- Live direct-message, contact-sync, group-discovery, and earlier membership
  projection paths have been verified. Controlled production-service tests of
  authoritative pruning, remote leave, startup backlog, and exactly-once
  behavior remain compatibility-evidence gaps. They gate claims for those
  service/client scenarios, not stable project status.
- Purple does not display or compare the numeric safety number. Acceptance is
  therefore a confirmation that the user completed verification through
  another trusted channel, not an in-plugin cryptographic comparison. This
  path is unit-tested but still needs a controlled live identity replacement.
- Outgoing attachments are cancellable while their upload is active, but are not
  restart-persistent like text messages. A crash or disconnect may require the
  user to send the file again.
- The bounded local-file read is synchronous on Purple's main thread.
  `O_NONBLOCK` prevents special-file open hangs but does not make regular-file
  I/O asynchronous, so a slow local or network-mounted file can pause the UI.
- The pinned Presage dependency owns contact-sync request and group-leave
  notification timestamps. Contact sync uses the local wall clock; group leave
  uses Signal's response timestamp with a local wall-clock fallback. These
  sends do not participate in signal-purple's per-core timestamp sequence.
- Presage and SQLx derive a separate SQLCipher connection-option copy which the
  pool retains for reconnects. That dependency-owned copy is outside
  signal-purple's zeroizing owner and requires upstream support to shorten or
  zeroize.
- SQLx does not expose join handles for its SQLite worker threads. Linux builds
  keep the Rust backend mapped until process exit to make bounded logical
  shutdown safe. True backend unloading after the last account closes requires
  upstream interruption and thread-joining support.
- Some pinned upstream error values are rendered into Purple errors and
  transient diagnostics. Reviewed signal-purple call sites do not directly add
  credentials or message bodies, but raw diagnostic output must be treated as
  private and sanitized before it is shared.
- Pidgin/libpurple 2 is a legacy in-process plugin environment. A memory-safety
  flaw in the UI or another plugin can access this process.
- Signal does not support third-party clients or promise protocol stability.
- Disappearing timers and remote deletions are not projected into Purple.
- Purple's buddy list stores synced contact aliases, group titles, canonical
  contact identifiers, and opaque group identifiers in plaintext. The plugin
  cannot prevent another in-process UI or plugin from retaining message text.
  Purple may also persist direct and group transcripts according to the user's
  global and per-conversation logging settings. Disabling future logging does
  not remove existing transcript files.

## Update response

Security and compatibility dependency updates require a full diff/provenance
review, Rust/C checks, store migration tests, and live non-production account
tests before release. Do not merge automated Signal-stack bumps solely because
they compile.
