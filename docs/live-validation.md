# Live validation

Live Signal compatibility is recorded per repository revision and test date.
A partial pass does not establish complete compatibility with the production
service.

## 2026-07-20 legacy-contact migration and idle validation

The installed ABI-v6 plugin was restarted against the normal desktop profile.
Before the fix, the connected Signal account had 49 snapshot-confirmed contacts
in the exact legacy `Signal` group, all created by an older plugin version and
missing the current managed marker. After one authoritative contact snapshot,
all 49 were marked and moved into the already-populated default `Buddies` group
alongside its 68 non-Signal buddies. No Signal contacts remained in `Signal`,
and all 11 managed Signal chats remained in `Chats`. Aggregate inspection found
no custom placements, local aliases, merged contacts, duplicates, or
cross-protocol entries in the migration source.

The Signal account remained connected while a 30-second post-restart sample
measured Pidgin at 0.03% of one CPU core and `signal-purple-core` at 0.00%. All
22 process threads were sleeping or waiting; the Signal core was blocked in
`do_epoll_wait`. The running process mapped the current installed plugin and
core inodes with no deleted-library mappings. The reported full-core idle spin
did not reproduce.

## 2026-07-20 basic-group implementation validation

The current implementation and automated tests cover stable opaque Purple
conversation identity, local-alias preservation, exact Storage Service record
completeness, current-state refresh of discovered and cached groups, atomic
active-member reconciliation, and send/join rejection outside the active set.
They also cover the confirmed remote-leave request and the rule that a managed
Purple chat is removed only after a successful completion.

Remote leave is implementation-tested, but it has not yet been exercised
against the production Signal service. The authoritative refresh/pruning path
also needs a controlled live mutation in which a group disappears or this
account leaves it. Purple's built-in **Remove Chat** and closing a conversation
remain intentionally local-only and are not substitutes for that test.

The previously installed ABI-v4 build was also sampled after a high-CPU report.
No full-core spin reproduced during the sample: Pidgin averaged about 0.5% CPU,
the Signal worker about 0.006%, and all threads slept normally. The sample did
confirm approximately 50 idle GLib wakeups per second from the old 20 ms event
poll. ABI v6 replaces that timer with descriptor-driven wakeups; the live check
above confirms the new worker sleeps while idle.

## 2026-07-19 isolated-profile run

Environment: Debian 13, Pidgin and libpurple 2.14.14, production Signal
service, and an isolated temporary Pidgin profile.

| Scenario | Result |
| --- | --- |
| Fresh linked-device QR flow | Passed |
| Encrypted-store disconnect and reconnect | Passed without a new QR |
| Messages queued while the linked device is offline | Implemented by the receive queue; not separately exercised live |
| Primary phone to plugin direct-message synchronization | Passed |
| Plugin to primary phone direct message | Passed |
| Incoming direct-message presentation | Reproduced in Pidgin: `PURPLE_MESSAGE_NO_LOG` forced informational-notice styling even with `PURPLE_MESSAGE_RECV`; fixed by retaining normal receive flags while conversation logging remains disabled |
| Durable message projection startup | Passed on the normal desktop profile: the SQLCipher projection schema initialized, the account reached connected state, and the versioned plugin/core pair loaded system-wide; an interrupted-message replay still needs a controlled live send |
| Identity-change policy | Store tests pass for uninterrupted unverified contacts, verified-contact send blocking, receive allowance, session reset, acceptance, and verification downgrade; controlled live replacement is not yet exercised |
| Outbound retry | Encrypted outbox persistence, completion, deferral, restart loading, bounded backoff, and identity-acceptance expediting are implemented and tested; forced live network loss is not yet exercised |
| Contact buddy-list creation and refresh | Passed with 46 visible contacts while Pidgin's offline filter remained disabled |
| Contact alias update and stale deletion | Implemented; stale-deletion decisions are unit-tested, but neither path was mutated live |
| Storage Service group discovery and restart reconciliation | Passed with 11 groups; three same-key copies produced by an earlier implementation were collapsed to 11 unique managed chats and another reconnect created no duplicates |
| Group title and active-membership projection | Passed; an opened 3-member group showed all members and one administrator flag |
| Group master-key confinement | Passed; persisted chats contained opaque `group-id` values and no raw group master keys |
| Group send/receive | Not exercised |
| Typing and receipts | Direct typing, delivery receipts, and focus-gated direct/group read receipts are implemented and unit/build tested; not exercised against another live client |
| Attachments | Group JPEG/PNG inline routing, decoder/dimension validation, image-store ownership, UI-retention fallback, and placeholder suppression pass focused C/Rust tests; a resend of the reported live group image is pending. Other direct/group attachments use bounded Purple transfers; another-client transfer remains to be exercised live |
| Second linked-device synchronization | Not exercised |

The installed plugin also passed the headless module probe. The live run proved
that explicitly requesting a Signal contact sync populates both the encrypted
store and Purple's buddy list. Contact snapshot reconciliation is covered by
deterministic C tests, including stale-entry removal and invalid snapshot input.
The same isolated account fetched 11 groups from Signal Storage Service without
requiring new group messages. Reconnect reconciliation, managed duplicate
cleanup, title persistence, chat opening, membership, and administrator flags
were inspected through Purple's live D-Bus API and persisted buddy list.
The normal desktop profile subsequently loaded the durable-projection build and
reconnected without relinking. Fork-level store tests cover upgrade bootstrap,
pending-message enumeration, and durable acknowledgment. A controlled incoming
send interrupted between Presage storage and Purple display remains necessary
before the exactly-once release gate can be checked.

## 2026-07-19 Debian 13 packaging run

Commit `9f68633` was archived and built twice from separate paths in a clean
Debian 13 amd64 root using the checksum-pinned upstream Rust 1.95 toolchain.
Both builds ran offline against the vendored Cargo graph and passed all three
C/Purple test executables. The source archives, runtime packages, and
debug-symbol packages were byte-identical:

| Artifact | SHA-256 |
| --- | --- |
| `signal-purple_0.1.0.orig.tar.xz` | `db908c615b5a737742500a0b97bbaaa33d62fad6215141ee7b36c8607459d4ab` |
| `signal-purple_0.1.0-1_amd64.deb` | `ac54b7eae073b184f6457fcb6135dbd50bf5aeb3730d1af8cf4cd7cb89ce8272` |
| `signal-purple-dbgsym_0.1.0-1_amd64.deb` | `dc0dbc31af7566045be2b486b15643a0ce80c5aa6240739d1956e5ff57b6a829` |

The generated DEP-5 file inventories all 385 vendored packages. Lintian
reported only the expected initial-upload warning. Installing the runtime
package back into the clean root, loading the installed plugin with the
headless probe, runtime dependency resolution, and the private
`$ORIGIN/signal-purple` core-library path all passed.

## 2026-07-19 hardening run

Commit `9f68633` passed Rust formatting, Clippy with warnings denied, 15 Rust
unit tests, the warning-as-error C build, all three C/Purple tests, and a second
C run under AddressSanitizer and UndefinedBehaviorSanitizer. GitHub reported no
open Dependabot, code-scanning, or secret-scanning alerts. The optimized ABI-v4
plugin was installed system-wide, Pidgin loaded both installed libraries, and
the existing account reconnected with 46 buddies and 11 group chats intact.
