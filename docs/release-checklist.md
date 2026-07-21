# 1.0 release checklist

This checklist is the release contract. A box is complete only when its result
is linked from the release issue or release pull request.

## Supported scope

- Debian 13 with libpurple 2.
- One Signal account per configured Purple account.
- Direct messages, contact synchronization, typing, delivery, read, retry, and
  messages received while this client was offline.
- Group discovery, title and membership synchronization, and group messaging.
- Identity-change warning and acceptance without relinking.
- Attachments within documented size and resource limits.
- Upgrade without losing the account, contacts, or trust state.

## Out of scope

- Signal registration, account recovery, or primary-device replacement.
- Calls, stories, payments, and unsupported Signal experiments.
- Operating systems or libpurple versions not named above.

## Build and supply chain

- [x] Release inputs are pinned and available without mutable Git references.
- [x] A clean Debian 13 environment produces the documented package.
- [x] Two builds produce identical binary package contents.
- [x] CI passes formatting, warnings, tests, ABI checks, and packaging checks.
- [ ] The source archive, package, checksums, SBOM, and signature agree.
- [x] No known unresolved release-blocking vulnerability remains.

## Interoperability

- [ ] Direct messages work both ways with supported Signal clients.
- [ ] Messages sent while signal-purple is offline arrive exactly once.
- [ ] Contact add, update, remove, and restart synchronization work.
- [ ] Group discovery, creation, membership changes, deduplication, and messages
  work across reconnects.
- [ ] Typing, delivery, read, failure, and retry states are accurate.
- [ ] Identity replacement blocks safely, warns, and resumes after acceptance.
- [ ] Attachment success, cancellation, rejection, and corruption are tested.

## Resilience and safety

- [ ] Network loss, reconnect, rate limits, and remote protocol errors recover.
- [ ] Corrupt state, unavailable key storage, and full disk fail safely.
- [ ] An idle connected account has no recurring backend poll wakeups or hot
  Pidgin/`signal-purple-core` thread.
- [ ] Sensitive values never appear in logs, crashes, or generated diagnostics.
- [x] ABI inputs have sanitizer and malformed-input coverage.
- [ ] Upgrade and rollback procedures preserve or explicitly migrate state.
- [ ] The release candidate completes its soak with no unresolved regression.

## Documentation and release

- [x] Installation, upgrade, rollback, relinking, and removal are documented.
- [x] Security boundaries, data retention, limitations, and support are current.
- [x] The release-please pull request matches the audited changelog and version.
- [ ] The signed release tag identifies the reviewed commit.
- [ ] Release artifacts are reproduced and smoke-tested from that tag.
- [x] A rollback decision and recovery path exist before publication.
