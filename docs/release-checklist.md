# 1.0 release checklist

This checklist is the release contract. A box is complete only when its result
is linked from the release issue or release pull request. Candidate-specific
evidence for the 0.2.0 pre-release belongs in
[validation tracker #5](https://github.com/adrighem/signal-purple/issues/5).
The candidate is the post-merge `main` commit that will be signed and tagged,
not the release pull-request head. Merging the release pull request establishes
that candidate but does not publish it.

For the explicitly labelled 0.2.0 pre-alpha, the release owner waived
exact-candidate live interoperability, network recovery, idle/diagnostic, and
soak evidence because dedicated non-production accounts were unavailable.
Those boxes remain unchecked and are known limitations, not test passes.

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
- [x] Primary CI passes formatting, warnings, tests, and ABI/module-load checks.
- [x] The candidate passes the clean Debian 13 build, test, and staged-install
  job.
- [x] The candidate's vendored source archive produces installable Debian
  packages twice with identical contents.
- [x] The source archive, package, checksums, SBOM, and signature agree.
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
- [x] Corrupt state, unavailable key storage, and full disk fail safely.
- [ ] An idle connected account has no recurring backend poll wakeups or hot
  Pidgin/`signal-purple-core` thread.
- [ ] Sensitive values never appear in logs, crashes, or generated diagnostics.
- [x] ABI inputs have focused malformed-input coverage.
- [x] The candidate C adapter passes AddressSanitizer and
  UndefinedBehaviorSanitizer.
- [ ] Upgrade and rollback procedures preserve or explicitly migrate state.
- [ ] The release candidate completes its soak with no unresolved regression.

## Documentation and release

- [x] Installation, upgrade, rollback, relinking, and removal are documented.
- [ ] Candidate install, load, upgrade, rollback, and uninstall paths pass for
  every advertised installation scope.
- [x] Security boundaries, data retention, limitations, and support are current.
- [x] The release-please pull request matches the audited changelog and version.
- [x] The signed release tag identifies the reviewed commit.
- [x] Release artifacts are reproduced and smoke-tested from that tag.
- [x] A rollback decision and recovery path exist before publication.
