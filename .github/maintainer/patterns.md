# Patterns

- Signal dependency changes require live compatibility evidence, not only CI.
- Purple calls stay on the GLib main thread.
- Provisioning URIs and user data never belong in logs or issue reports.
- Destructive reconciliation needs explicit snapshot boundaries. Never infer a
  deletion from a partial or failed Signal store read.
- A linked device must explicitly request contact synchronization. Draining the
  initial message queue may yield only the device's own contact.
- Candidate evidence must identify the post-merge commit that will be tagged.
  A pull-request head is insufficient when the merge method creates a distinct
  commit or when artifact metadata depends on commit timestamps.
- A release pull request must not auto-close its validation tracker; the
  tracker remains open through candidate validation, packaging, signing, and
  publication evidence.
- Manually replacing Release Please's generated pull-request body can prevent
  it from recognizing the merged release PR. With tagging intentionally
  deferred, the next workflow run may open a premature next-version PR that
  repeats all commits since the last tag.
- With global `push.default=tracking`, a local branch created from
  `origin/main` can retain `refs/heads/main` as its upstream. Always inspect
  the upstream and use an explicit `HEAD:refs/heads/<branch>` refspec when a
  feature-branch push is intended.
- A human-approved release-test waiver is not a pass. Keep the corresponding
  checklist gates unchecked and carry the unverified scope into permanent
  validation records and release notes.
- Release helpers that accept Git revisions must peel an annotated tag to one
  verified commit before deriving timestamps, versions, or archive contents.
  Test commit and signed/annotated-tag inputs for byte-identical output.
- Cargo Dependabot updates can remove the signal-core
  `# x-release-please-version` marker from `Cargo.lock`. Treat that as a
  release blocker and guard the marker in CI before merging automated lockfile
  changes.
- Manually listing Rust modules as CMake custom-command dependencies can leave
  incremental CMake builds using a stale backend after a new module is added.
  Invoke Cargo as the dependency authority and let its incremental build
  decide whether compilation is needed.
- Keep contributor commands and primary CI phases behind one repository-owned
  driver. Duplicated command lists drift in warning policy, test scope, and
  install probing even when each list is individually reasonable.
- Verify live GitHub branch, tag, and release controls whenever release trust
  depends on them. A documented protected-branch policy does not protect an
  unconfigured repository.
- A payload limit checked only after a dependency returns a complete buffer is
  not an allocation limit. Enforce the bound while reading or downloading,
  including when sender-controlled size metadata is absent or understated.
- Empty contact snapshots must not erase a usable profile-derived display name.
  Purple local aliases remain user-owned and take precedence over server data.
- Optional profile enrichment must not delay message delivery or outlive the
  receive queue boundary. Coalesce it per contact, bound network waits, and
  serialize the final merge with synchronized contact writes.
- Purple's image store can animate decoder-supported image formats but cannot
  represent attached video. Signal may flag an MP4 as GIF-style media; keep
  that payload in the bounded file-transfer path unless a separately reviewed
  UI or transcoding design can preserve resource and plaintext-cache policy.
- When conversion is justified by protocol metadata, keep the media decoder in
  a disposable child with fixed arguments, an empty environment, pipe-only
  I/O, OS and application resource limits, strict output validation, and the
  original attachment as the failure presentation. Do not link an untrusted
  media decoder into the plugin process for presentation convenience.
- A single-connection async database pool prevents SQLite write races but does
  not make every actor-level overlap harmless. Gate optional startup work behind
  the dependency's drained-queue boundary when its receive initialization can
  retain the sole connection across longer protocol operations.
- A long-lived stream can retain an in-progress future after `select!` chooses
  another branch. If that future owns an exclusive resource needed by the
  winning branch, awaiting the branch inline self-deadlocks. Keep the stream
  independently scheduled and bound the handoff queue.
- A module probe which links production helpers into an export-dynamic test
  executable can silently interpose those helpers over the loaded module's own
  copies. Bind internal module calls locally and inspect dynamic relocations so
  the test proves which implementation it exercises.
- A handler's event-loop continuation result is not evidence that a payload was
  accepted. Track projection acceptance separately so malformed durable input
  can remain replayable without tearing down an otherwise healthy event source.
- Before splitting an actor whose behavior is gated by several related
  booleans, replace them with a small tested transition model. Invalid state
  combinations then become unrepresentable and later module boundaries can
  consume named readiness and authority predicates.
- Make integration probes construct state through the production owner instead
  of copying its initializer. Keep framework callback detachment as an explicit
  phase before the shared finalizer, and observe that ordering from a callback
  which runs during teardown.
- Language-local ABI tests can each pass while their declarations disagree.
  Link one side to the production library and compare compiled constants,
  layouts, and public limits across the actual boundary.
- Keep user data-lifecycle instructions out of maintainer release automation;
  link the release gates to the user procedure they validate.
- A formatting gate must pin the formatter major and share one style between
  check and fix modes. Scan source directories rather than Git state so release
  archives receive the same result as working trees.
