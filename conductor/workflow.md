# Project Workflow

## Guiding Principles

1. The active track plan is the source of truth.
2. Use test-driven changes for every reproduced defect.
3. Preserve the documented C/Rust ownership boundary.
4. Run commands that may capture inherited state under a minimal `env -i`
   allowlist.
5. Do not use live Signal credentials or production account data in tests.

## Task Workflow

For each task in `plan.md`:

1. Change its marker from `[ ]` to `[~]`.
2. Add focused regression coverage and run it to confirm the expected failure.
3. Implement the smallest coherent fix.
4. Run the focused test, then the relevant Rust or C suite.
5. Refactor only while tests remain green.
6. Update architecture, security, or development documentation when behavior
   changes.
7. Commit the task with a Conventional Commit message.
8. Attach a Git note describing the task, changed files, and rationale.
9. Mark the task `[x]` and append the first seven commit characters.
10. Commit the plan update.

All commits and test commands must avoid credentials and complete environment
dumps. Issue-related commits use a `Refs #NN` footer when an issue number is in
scope.

## Phase Completion Verification and Checkpointing Protocol

Each phase ends with the required manual-verification task:

1. Determine the phase diff and verify every changed code path has focused
   tests.
2. Announce and run `scripts/check.sh full` under the sanitized environment.
3. Present manual steps and expected outcomes based on `product.md`,
   `product-guidelines.md`, and the track plan.
4. Pause for explicit user confirmation.
5. Create a checkpoint commit and attach a Git note containing the automated
   command, manual steps, and user confirmation.
6. Add `[checkpoint: <sha>]` to the phase heading and commit that plan update.

## Quality Gates

- Focused regression tests pass.
- The complete canonical gate passes.
- C and Rust formatting and static analysis pass on the pinned toolchain.
- New ownership and cancellation paths have explicit tests.
- Documentation matches user-visible behavior.
- No sensitive values appear in output, fixtures, commits, or notes.

## Development Commands

### Focused Rust

```sh
env -i HOME=/home/vincent PATH=/usr/local/bin:/usr/bin:/bin \
  /usr/bin/cargo test --locked --manifest-path rust/signal-core/Cargo.toml
```

### Focused C

```sh
env -i HOME=/home/vincent PATH=/usr/local/bin:/usr/bin:/bin \
  ctest --test-dir build --output-on-failure
```

### Complete Gate

```sh
env -i HOME=/home/vincent PATH=/usr/local/bin:/usr/bin:/bin \
  scripts/check.sh full
```

## Definition of Done

A task is done when its regression is demonstrated, implementation and
documentation are complete, relevant suites pass, the change is committed, a
Git note records the rationale, and the plan references the commit.
