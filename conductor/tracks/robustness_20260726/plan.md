# Track Plan

## Phase 1: Reliability and lifecycle hardening

- [~] Task: Make display acknowledgements and attachment cancellation reliable
  - [ ] Sub-task: Add queue-pressure and cancellation-state regression tests
  - [ ] Sub-task: Separate acknowledgement delivery from bounded work admission
  - [ ] Sub-task: Make attachment cancellation capacity-independent
- [ ] Task: Close outgoing transfer lifetime and local file-admission gaps
  - [ ] Sub-task: Add pending-transfer disconnect coverage
  - [ ] Sub-task: Track every outgoing transfer from creation
  - [ ] Sub-task: Add and use a bounded single-open regular-file reader
- [ ] Task: Centralize timestamps and make blocking backend phases cancellable
  - [ ] Sub-task: Add monotonic timestamp and shutdown-boundary tests
  - [ ] Sub-task: Route locally generated message timestamps through one allocator
  - [ ] Sub-task: Apply shutdown selection to synchronization and projection waits
- [ ] Task: Minimize credential lifetime and harden the validation gate
  - [ ] Sub-task: Move the passphrase immediately into zeroizing ownership
  - [ ] Sub-task: Drop the passphrase immediately after store opening
  - [ ] Sub-task: Make CTest fail when no tests are discovered
- [ ] Task: Update architecture and security documentation
  - [ ] Sub-task: Document reliable control paths and bounded local file admission
  - [ ] Sub-task: Document timestamp and shutdown invariants
- [ ] Task: Conductor - User Manual Verification 'Reliability and lifecycle hardening' (Protocol in workflow.md)
