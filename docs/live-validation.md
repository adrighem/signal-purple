# Live validation

Live Signal compatibility is recorded per repository revision and test date.
A partial pass does not establish complete compatibility with the production
service.

## 2026-07-21 0.2.0 pre-release validation

The signed 0.2.0 pre-release commit is
`59d4f257a3b2514261d3fc773da4e9df90d9ffd4`. Its
[main CI](https://github.com/adrighem/signal-purple/actions/runs/29845632084)
and [CodeQL](https://github.com/adrighem/signal-purple/actions/runs/29845630422)
passed, including all 40 Rust tests, all four C/Purple tests, and the
annotated-tag archive regression. The rebuilt runtime package, debug package,
and probe are byte-identical to the original candidate, so its four GCC
AddressSanitizer/UndefinedBehaviorSanitizer tests and all nine tests from the
pinned Presage store revision transfer unchanged.
The sanitizer run covered the C adapter with LeakSanitizer disabled; it did
not independently instrument the Rust library.
At freeze time, GitHub reported no open Dependabot, code-scanning, or
secret-scanning alerts.

The vendored source archive was generated twice independently and matched
byte-for-byte. Two clean Debian 13 amd64 package builds used the pinned image
and checksum-verified Rust 1.95 toolchain with networking disabled. Their
runtime and debug packages were byte-identical. Both builds passed packaged
tests, installation, module probing, private-backend resolution, and removal.
The only archived changes from the initial candidate were corrected
release-process documentation plus the reviewed archive helper, CI wiring,
and regression test.

| Artifact | SHA-256 |
| --- | --- |
| `signal-purple_0.2.0.orig.tar.xz` | `9e98442119df4d8991e9eee4553c6b882c08c47bb677a6db4984a5ac1545011b` |
| `signal-purple_0.2.0-1_amd64.deb` | `19190da4933fa6310def64112ec6325ae499cf75b7d16b23aaf9f92346368f82` |
| `signal-purple-dbgsym_0.2.0-1_amd64.deb` | `ba98233728fc98c985db195fa758490ed4b3d3987ece24835e15981a51a9c26f` |
| `signal-purple-0.2.0.spdx.json` | `c2ecc09e59fd9d2ed7deaeeea556d76c5c18bff0faa853d465e9ea84276347b6` |
| `SHA256SUMS` | `7092498a7a9060d8414bf3a2d83f43f763ec4a80efa29cf573ad8c6ff72e0cce` |
| `SHA256SUMS.asc` | `ffda6563e6b4387bcbf0d62033844bb7cc92c0e089e66a592cbf91d70492a663` |

The rebuilt runtime package, debug package, and probe were byte-identical to
the initial candidate, so its Debian-package and CMake source-install lifecycle
evidence transfers unchanged. It covered fresh installation, 0.1.0 to 0.2.0
upgrade, rollback, re-upgrade, probing, and removal while preserving
profile-state sentinels. A negative control showed that installing an older
CMake build directly over newer files can be skipped by timestamp checks.
Manifest-first replacement passed and is now required by the release process.
No real linked account or encrypted database migration is claimed by this
lifecycle evidence. The runs covered system Debian-package paths and one
disposable custom CMake prefix, not every documented prefix variant.

Corrupt encrypted state, an unavailable Secret Service, and runtime ENOSPC
fault injection failed closed. Three full-disk runs rolled back the failed
write, preserved and reopened the encrypted store, accepted a recovery write,
and excluded synthetic paths, account values, recipients, message bodies, and
passphrases from returned diagnostics.

Dedicated non-production Signal accounts were unavailable. The release owner
therefore waived exact-candidate live interoperability, network recovery,
idle/diagnostic capture, and soak validation for this explicitly labelled
pre-alpha. These checks were not run and remain unverified.
No official Signal client version was tested. Production-service compatibility
is not established for this release.

The signed annotated tag [`v0.2.0`](https://github.com/adrighem/signal-purple/releases/tag/v0.2.0)
targets the reviewed commit above, and GitHub verified its signature. The tag
and `SHA256SUMS` signature use key fingerprint
`B3C0 B75F A3B3 3AC2 7873 8C5C B179 8BCD A760 54BD`. Generating the source
archive directly from the tag reproduced the published archive byte-for-byte;
a clean network-disabled Debian 13 build from that archive passed all four
tests, installation, module probing, private-backend resolution, and removal.
All six downloaded pre-release assets matched the locally verified files, and
the downloaded checksum manifest and detached signature verified successfully.

## 2026-07-21 reconnect group-routing regression

The headless Purple probe now reproduces Pidgin's reconnect-time title reset:
an existing managed group conversation receives a membership update while the
account is still connecting, and Pidgin's title autoset falls back to the
opaque canonical group identifier. The adapter restores the saved local title
after the membership refresh. The regression asserts that the conversation
remains a chat, retains its stable internal identifier, uses the friendly
display title, and does not create a direct-message conversation.

Rust routing tests also distinguish locally authored durable rows from incoming
messages, and successful plugin sends mark their stored text or attachment row
as projected without retrying the network if that bookkeeping fails. The
Release build, all 40 Rust tests, all four C/Purple tests, and the C adapter
under AddressSanitizer and UndefinedBehaviorSanitizer passed. LeakSanitizer was
disabled for the full Purple probe because process-global libpurple and media
registries retain allocations at shutdown.

The normal encrypted store had no pending message for the reported group when
inspected. Commit `822414a` was then installed system-wide and loaded from the
expected plugin and private backend paths. The normal profile reconnected with
49 contacts and 10 groups, reported no Signal plugin error, and retained one
saved copy of the reported chat in `Chats` with a friendly alias. A 30-second
post-startup sample averaged 0.033% of one CPU core. A controlled production
replay still requires a new group message to arrive while the account is
offline.

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
