# Live validation

Live Signal compatibility is recorded per repository revision and test date.
A partial pass does not establish complete compatibility with the production
service.

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
| Contact buddy-list creation and refresh | Passed with 46 visible contacts while Pidgin's offline filter remained disabled |
| Contact alias update and stale deletion | Implemented; stale-deletion decisions are unit-tested, but neither path was mutated live |
| Storage Service group discovery and restart reconciliation | Passed with 11 groups; three same-key copies produced by an earlier implementation were collapsed to 11 unique managed chats and another reconnect created no duplicates |
| Group title and active-membership projection | Passed; an opened 3-member group showed all members and one administrator flag |
| Group master-key confinement | Passed; persisted chats contained opaque `group-id` values and no raw group master keys |
| Group send/receive | Not exercised |
| Typing and delivery receipts | Not exercised |
| Second linked-device synchronization | Not exercised |

The installed plugin also passed the headless module probe. The live run proved
that explicitly requesting a Signal contact sync populates both the encrypted
store and Purple's buddy list. Contact snapshot reconciliation is covered by
deterministic C tests, including stale-entry removal and invalid snapshot input.
The same isolated account fetched 11 groups from Signal Storage Service without
requiring new group messages. Reconnect reconciliation, managed duplicate
cleanup, title persistence, chat opening, membership, and administrator flags
were inspected through Purple's live D-Bus API and persisted buddy list.

## 2026-07-19 Debian 13 packaging run

Commit `2837e31` was built twice from separate paths in a clean Debian 13
amd64 root using the checksum-pinned upstream Rust 1.95 toolchain. Both builds
ran offline against the vendored Cargo graph and passed all three C/Purple test
executables. The source archives, runtime packages, and debug-symbol packages
were byte-identical:

| Artifact | SHA-256 |
| --- | --- |
| `signal-purple_0.1.0.orig.tar.xz` | `bc31796da82d768390e70852f1d82aee689e0c71ceb8fba8924e521925de7f63` |
| `signal-purple_0.1.0-1_amd64.deb` | `05585febca056af1adb5c91612669b49265cc6bdf17b02e2e7ff4a56b2355251` |
| `signal-purple-dbgsym_0.1.0-1_amd64.deb` | `b498aabb25f813256ce4eff5bc501d6ad700c1fc4af248072279a5e30364ba16` |

The generated DEP-5 file inventories all 385 vendored packages. Lintian
reported the expected initial-upload warning and one informational Rust
FORTIFY note, with no package error. Runtime dependency resolution and the
private `$ORIGIN/signal-purple` core-library path were also verified.
