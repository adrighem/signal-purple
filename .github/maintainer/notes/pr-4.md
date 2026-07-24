# PR #4: release 0.2.0

- **Intent:** apply the generated 0.2.0 manifest, package, citation, lockfile,
  version, and changelog updates.
- **Provenance:** same-repository Release Please branch. The generated commit is
  GitHub-verified; the owner's transparent changelog correction is unsigned.
- **Scope:** six text files, 28 additions and five deletions. No runtime code,
  workflows, permissions, dependencies, binaries, install scripts,
  credentials, or network behavior change. The lockfile changes only the root
  package version.
- **Validation:** CI, dependency review, and CodeQL pass at `a0d80f8`; the
  optional managed AI scan failed only because GitHub selected an unsupported
  model. No exact-candidate live, soak, recovery, reproducibility, or artifact
  evidence is recorded.
- **Blockers:** PR #6 is not in the candidate; PR #4's body would auto-close
  ISSUE:5; the body still advertises a nonexistent `v0.1.0` comparison; and the
  0.2.0 changelog repeats all nine 0.1.0 entries without an explicit cumulative
  release-note policy.
- **Candidate identity:** merging creates a different commit from the current
  PR head, while source archives include commit timestamps. Freeze and validate
  the resulting `main` commit rather than the current head.
- **Recommendation:** do not merge the current revision. Merge the corrected
  PR #6 first, let Release Please refresh this PR, re-review the full updated
  diff and body, then use this PR as the source of the release-version change.
- **Confidence:** high.
- **Public action:** none; edits or merge require human approval.
- **Outcome:** after approval, PR #6 was merged into the release branch, the
  changelog was curated as cumulative first-public-release notes, and the PR
  body was changed so it no longer closed ISSUE:5. All refreshed checks passed
  at `66791a9`.
- **Merged:** `c14e3551dec9942d83cadeeafa5344c16c439c1a`. This is the frozen
  0.2.0 candidate; no tag or GitHub release was created.
