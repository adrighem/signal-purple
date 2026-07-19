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
| Contact buddy-list creation and refresh | Passed with 46 visible contacts while Pidgin's offline filter remained disabled |
| Contact alias update and stale deletion | Implemented; stale-deletion decisions are unit-tested, but neither path was mutated live |
| Group send/receive | Not exercised |
| Typing and delivery receipts | Not exercised |
| Second linked-device synchronization | Not exercised |

The installed plugin also passed the headless module probe. The live run proved
that explicitly requesting a Signal contact sync populates both the encrypted
store and Purple's buddy list. Contact snapshot reconciliation is covered by
deterministic C tests, including stale-entry removal and invalid snapshot input.
