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
