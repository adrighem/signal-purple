# Track Plan

## Phase 1: Reliability and lifecycle hardening

- [x] Task: Make display acknowledgements and attachment cancellation reliable 374a92a
  - [x] Sub-task: Add queue-pressure and cancellation-state regression tests
  - [x] Sub-task: Add a coalescing acknowledgement inbox with shutdown draining
  - [x] Sub-task: Put cancellation state directly in attachment admission permits
  - [x] Sub-task: Document the reliable control-path invariants
- [x] Task: Make the C validation gate fail closed c7c7bc8
  - [x] Sub-task: Enable C tests explicitly in the canonical build
  - [x] Sub-task: Make CTest fail when no tests are discovered
- [~] Task: Close outgoing transfer lifetime and local file-admission gaps
  - [ ] Sub-task: Add pending-transfer disconnect coverage
  - [ ] Sub-task: Track every outgoing transfer from creation
  - [ ] Sub-task: Add and use a bounded single-open regular-file reader
  - [ ] Sub-task: Document transfer ownership and local file admission
- [ ] Task: Centralize locally generated message timestamps
  - [ ] Sub-task: Add concurrent and clock-rollback timestamp tests
  - [ ] Sub-task: Route locally generated message timestamps through one allocator
  - [ ] Sub-task: Keep wall-clock retry deadlines separate from protocol timestamps
  - [ ] Sub-task: Document the monotonic timestamp invariant
- [ ] Task: Make blocking backend phases cancellable
  - [ ] Sub-task: Inventory synchronization, replay, outbox, and download waits
  - [ ] Sub-task: Add bounded worker-shutdown regression tests
  - [ ] Sub-task: Apply the shutdown boundary to every inventoried wait
  - [ ] Sub-task: Document the worker shutdown boundary
- [ ] Task: Minimize credential lifetime
  - [ ] Sub-task: Add constructor and worker credential-lifetime coverage
  - [ ] Sub-task: Move the passphrase immediately into zeroizing ownership
  - [ ] Sub-task: Drop the passphrase immediately after store opening
  - [ ] Sub-task: Document the credential ownership boundary
- [ ] Task: Audit architecture and security documentation for consistency
  - [ ] Sub-task: Check ABI and status-value documentation against regression coverage
  - [ ] Sub-task: Check ownership, cancellation, timestamp, shutdown, and credential docs
- [ ] Task: Conductor - User Manual Verification 'Reliability and lifecycle hardening' (Protocol in workflow.md)
