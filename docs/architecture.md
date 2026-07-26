# Architecture

signal-purple deliberately owns the boundary between Purple and the evolving
Signal client ecosystem.

## Components

`libsignal-purple.so` is a Purple 2 protocol plugin written in C. It owns Purple
accounts, conversations, buddy/group mapping, QR presentation, libsecret access,
and all GLib lifecycle work. It never performs Signal cryptography.

`libsignal_core.so` is a Rust `cdylib`. Each Purple account creates one opaque
core with a dedicated OS thread, a current-thread Tokio runtime, and a
`LocalSet`, matching Presage's non-`Send` runtime constraints. It owns network,
crypto, storage, linking, sync, and message normalization.

Presage, libsignal-service-rs, and libsignal are exact Git revisions recorded in
the lockfile. Purple never calls their unstable interfaces directly.

## ABI and ownership

[`include/signal_core.h`](../include/signal_core.h) is the only C/Rust contract.
It is versioned and exposes opaque cores, asynchronous commands, a borrowed
event-notifier descriptor, nonblocking event polling, and explicit event
destruction.

- C strings passed into a command are validated and copied before return.
- Rust owns all event strings and blobs until `signal_event_free`.
- The event queue is bounded at 4096 entries. Overflow produces a fatal event
  and requires reconnecting so data is resynchronized instead of silently
  dropping an arbitrary message.
- Ordinary outbound work uses a bounded command queue. Display acknowledgements
  use a separate coalescing inbox whose registered IDs are bounded by pending
  projections, so queue pressure cannot lose durable UI acceptance. Attachment
  cancellation is recorded synchronously on the admitted request, independent
  of command capacity.
- Fallible exported operations catch panics at the FFI boundary. Teardown is
  deliberately written from non-panicking primitives so the worker is always
  joined before its allocation is freed.
- A private nonblocking descriptor becomes readable after Rust queues an event.
  GLib watches it and drains bounded event batches, so an idle account has no
  recurring polling timer and incoming-event latency does not depend on a
  backoff interval.
- Teardown destroys the descriptor source, cancels every admitted attachment,
  sends shutdown, joins the worker, then frees the core. The worker aborts
  contact synchronization and attachment tasks before draining accepted
  projection acknowledgements. Profile lookup, group synchronization, durable
  replay, outbox retry, and live message projection all race the authoritative
  shutdown signal, as do store registration and schema initialization. Parent
  directory setup completes synchronously before the worker is created, so no
  plugin-owned filesystem operation can outlive the joined actor. Cleanup gets
  a two-second budget, and Tokio runtime shutdown gets a separate two-second
  budget, so synchronous core teardown does not wait forever for dependency
  work. Dropping a timed-out future stops the actor from polling it, but an
  already dispatched SQLx SQLite operation can finish on its dependency-owned
  thread afterward. Linux builds therefore mark `libsignal_core.so` with ELF
  `NODELETE`, verified by staged-install and release probes, so dependency code
  remains mapped until process exit. Those threads never call C or Purple.
  Cancelling projection, including an attachment download or an
  acknowledgement left after the cleanup budget, leaves the content eligible
  for replay. Cancelling an outbox retry leaves its encrypted row available for
  the next connection.

## Connection sequence

1. Purple resolves an account-specific database path.
2. The plugin loads or generates its SQLCipher passphrase through libsecret.
   The C-owned secret is freed as soon as the Rust constructor returns.
3. The Rust FFI copies the passphrase directly into a non-debuggable,
   zeroizing owner. The worker consumes that owner while opening SQLCipher and
   wipes it before checking registration or starting the session.
4. Presage serializes the SQLx pool through one SQLite connection. Startup
   registration refresh, contact sync, replay, receipts, acknowledgements, and
   outbox work can still progress concurrently at the actor level, but their
   database operations do not race each other into `SQLITE_BUSY` within one
   live pool. This avoids retrying a whole Signal send after its remote side
   effect may already have happened. Rapid reconnect and a separate core or
   process using the same store remain outside this serialization boundary.
5. An existing linked device loads immediately. A fresh store starts Presage's
   secondary-device provisioning and emits a QR PNG.
6. The backend starts the receive stream and processes queued sync/session data.
7. At the first `QueueEmpty`, the backend reads the account's Storage Service
   manifest, verifies the exact returned record-key set, and refreshes the union
   of manifest-discovered and cached groups from current GroupsV2 state. A group
   is active only while the linked account's own ACI is a current member. Only a
   definitive inaccessible/deleted response or decrypted nonmembership permits
   pruning. Network, authentication, decoding, completeness, or database errors
   leave the entire prior set intact rather than applying a partial update.
8. The core emits the contact snapshot and, after a successful refresh, the
   authoritative group snapshot before becoming ready. If group refresh fails,
   the account still connects for direct messaging, but group operations stay
   unavailable while an in-session retry runs on a bounded interval.

This ordering prevents sends before queued profile, session, and sender-key
updates have been applied. The queue contains envelopes addressed to this
linked device, including messages Signal still has queued after it was offline.
It is not a general conversation-history API and cannot retrieve arbitrary
older messages from the primary phone or Signal service.

## Message mapping

- Canonical Signal service identifiers are Purple buddy names. Synced profile
  names are server aliases only. Explicit snapshot boundaries let Purple apply
  contact creates and updates before removing stale managed entries. User-made
  buddies without the managed marker are never swept. The backend explicitly
  requests a contact sync after opening the receive stream, then refreshes the
  projection when Presage reports synchronized contacts. Because Signal does
  not expose presence, contacts are marked reachable while the linked account
  is connected so Purple's default offline filter does not hide them.
- Group master keys remain private 32-byte values in the encrypted backend
  store. Purple receives a domain-separated SHA-256 identifier for persistence,
  joining, and its internal conversation name. The Signal group title is
  presentation data, so duplicate titles cannot collide and a user-set local
  alias is not overwritten by a later Signal title refresh. Rust resolves the
  opaque identifier only against the current authoritative active-group set
  before a join, text send, or attachment send. Snapshot reconciliation removes
  stale plugin-managed entries and collapses duplicate managed chats. Each
  connection assigns a collision-free sequential Purple chat integer.
- Incoming text is markup-escaped. Outgoing Purple markup is stripped.
- Own-device `SynchronizeMessage` values render as outgoing messages.
- Delivery receipts are sent when Presage marks an envelope as needing one.
- Read receipts are held until Purple reports that the direct or group
  conversation is focused. Background delivery and notification rendering do
  not mark a message read.
- Direct and group sends are written to an encrypted outbox before network
  submission. Failed entries retain their original Signal timestamp and retry
  with bounded exponential backoff across reconnects. Accepting a verified
  identity change immediately expedites that contact's queued messages.
- Adapter-generated text, attachment, typing, delivery-receipt, and read-receipt
  sends share one timestamp allocator per core. It uses the wall clock as a
  floor and atomically advances beyond the last allocation, so concurrent sends
  and clock rollback cannot reuse a timestamp. Durable message retries retain
  their original timestamp, while retry deadlines continue to use the wall
  clock directly.
- Incoming direct and group attachments are downloaded on the backend thread
  and copied across the owned ABI. An incoming group JPEG or PNG whose declared
  MIME type matches its file signature, passes decoder validation, and remains
  within an 8192-pixel edge and 16-megapixel limit is copied into Purple's image
  store and written to the originating chat with the Signal sender and
  timestamp. If the UI does not retain the image, or validation fails, the
  attachment falls back to Purple's receive-file flow. Outgoing transfers use
  Purple's direct and group send-file callbacks. Each admitted request carries
  cancellation state from queue admission through its backend upload task, so a
  cancellation which overtakes a queued send still prevents the upload.
  The C adapter registers every outgoing transfer when it is created and
  holds a temporary reference across synchronous Purple start callbacks. On
  disconnect it cancels started transfers, then severs every remaining live
  transfer from its context before freeing the connection, including transfers
  still waiting in a file chooser. A late acceptance is cancelled locally.
  Local files are opened once with `O_NONBLOCK` to avoid special-file open
  stalls, inspected through that descriptor, restricted to regular files, and
  read only up to the configured limit plus one byte.
  Each file is capped at 25 MiB and each incoming message at 50 MiB. Outgoing
  admission spans queued, recovery-deferred, and active work, with at most two
  files totaling 50 MiB per account. Queued binary events and unresolved Purple
  receive prompts each have independent 64 MiB budgets. The
  [attachment policy](attachment-policy.md) maps these limits to their owners.
  Decrypted attachment data is never written to a plugin-managed plaintext cache.
  Attachment sends are not part of the durable text-message outbox.
- Presage acknowledges an envelope to Signal before the Purple UI can display
  it, but saves supported content in SQLCipher first. signal-purple records a
  separate encrypted projection acknowledgment only after Purple accepts the
  corresponding event. A crash anywhere between network receipt and UI
  delivery therefore leaves the message eligible for replay on reconnect.
  Acknowledgements are coalesced by registered delivery ID, retried after local
  store failures, and drained within the bounded orderly-shutdown budget.
  Existing stored history is marked projected when this mechanism is first
  initialized, preventing an upgrade from flooding conversations.
- Purple 2 has no robust per-message receipt update API, so received receipts
  are currently consumed without a misleading UI projection.

## Group lifecycle actions

The chat-node action **Leave Signal group…** is deliberately separate from
Purple's generic removal UI. After confirmation, the C plugin submits an
asynchronous leave command. The backend removes this account from the Signal
group and reports completion; only a successful completion closes the managed
conversation and removes its chat-list node. A failure leaves the local node in
place so the UI does not claim a leave which the service rejected.

Purple 2 does not expose a protocol-plugin callback for its built-in **Remove
Chat** operation. It therefore removes only the local buddy-list node, which can
return in the next authoritative snapshot. Closing a conversation tab is also
local state and does not change Signal membership.

| Pidgin action | Signal effect |
| --- | --- |
| Open or join a saved chat | Opens the local conversation; it does not join a Signal group. |
| Send a message or file | Sends only after the refreshed state confirms current membership. |
| Rename or move a saved chat | Changes only its local alias or buddy-list placement. |
| Close the conversation tab | Local-only; membership is unchanged. |
| Built-in **Remove Chat** | Removes the local node only; it may return on sync. |
| **Leave Signal group…** | After confirmation, removes this account remotely and removes the managed local node only after acceptance. |

Pidgin/Purple 2 exposes no compatible protocol actions here for inviting or
removing other members, changing roles, editing group attributes, approving
join requests, or managing invite links. Those Signal operations are not mapped
or advertised by this basic group implementation.

## Identity replacement

Rejecting every changed identity appears safe but can lose inbound messages:
Signal's websocket envelope is acknowledged before Presage finishes decrypting
and before Purple can warn the user. Trusting every replacement, as the pinned
Flare store does, avoids that loss but removes the protection expected for a
verified contact.

signal-purple records replacement keys and verification state inside SQLCipher.
Receiving always continues. Unverified contacts continue with a one-time
advisory. Explicitly verified contacts are blocked only for sending and expose
an acceptance action on that buddy. Acceptance installs the pending key, clears
sessions and sender keys, and downgrades the contact to unverified. This keeps
normal chats uninterrupted while retaining an explicit safety boundary where
the user previously chose one.

## Flare comparison

Flare also uses Presage but presents contacts and groups as
conversation threads instead of a presence-oriented buddy list. Its UI exposes
a manual
[`sync-contacts` action](https://gitlab.com/schmiddi-on-mobile/flare/-/blob/484450e4cf8a34992a68df753a872e530a5b3d2c/src/gui/window.rs#L353)
that delegates to Presage's contact request.
[After the receive queue is empty](https://gitlab.com/schmiddi-on-mobile/flare/-/blob/484450e4cf8a34992a68df753a872e530a5b3d2c/src/backend/manager.rs#L106),
Flare initializes channels from its local thread store. Its
[`contacts()` projection](https://gitlab.com/schmiddi-on-mobile/flare-backend/-/blob/8f9f178cb5ec9040d73fdd7c70a3ca3a5bcdcb72/flare-store/src/lib.rs#L133)
also enriches synchronized contact names with stored Signal profiles.

Flare loads stored history when a conversation is opened, so content saved by
Presage remains available after a restart. It does not maintain a durable
frontend-delivery acknowledgment, and its backend documentation leaves retry
functionality as future work. signal-purple instead replays only content which
Purple has not acknowledged, because loading a full stored timeline into a
libpurple conversation would create duplicates and expose historical messages
beyond the linked device's normal delivery and crash-replay boundary.

signal-purple applies the same essential contact-request step automatically on
every connection. It then reconciles complete snapshots into plugin-managed
Purple buddies. Because Purple normally hides offline buddies and Signal has no
presence API, synchronized contacts are marked reachable while the account is
connected. Contact names and synchronized phone numbers are used as aliases;
profile enrichment for contacts and group-only members is not implemented yet.
The registered account's own Signal profile name is retrieved as a fallback
for group participant display when no local Purple account alias is set. For
groups, signal-purple's pinned Presage fork adds authoritative Storage Service
refresh and pruning plus the remote group-leave operation, so the chat list is
complete without waiting for each group to receive a new message and contains
only active memberships.

## Deliberate boundaries

The first version does not implement in-plugin safety-number comparison,
primary registration, contact discovery, calls, or official backup
compatibility. It also does not project disappearing timers or remote deletion
into Purple.
The pinned Presage dependency owns contact-sync request and group-leave
notification timestamps. Contact sync uses the local wall clock; group leave
uses Signal's response timestamp with a local wall-clock fallback. These sends
do not participate in signal-purple's per-core timestamp sequence and are not
covered by its uniqueness guarantee.
The adapter leaves conversation logging to Purple. New direct and group
conversations inherit libpurple's `log_ims` and `log_chats` defaults; the
frontend may then apply a saved conversation-specific choice. Reusing a
conversation or sending through the PRPL preserves its current runtime setting.
Messages keep normal send/receive flags, without `PURPLE_MESSAGE_NO_LOG`, so an
enabled conversation is logged normally. Purple owns those transcript files
outside signal-purple's SQLCipher store. Synced buddy aliases and identifiers
also live in Purple's plaintext buddy list.
