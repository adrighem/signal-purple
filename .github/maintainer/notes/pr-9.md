# PR #9: support annotated source archive tags

- **Intent:** fix signed-tag source reproduction before publishing 0.2.0.
- **Cause:** `git show -s --format=%ct v0.2.0` included annotated-tag payload
  and signature text before the commit epoch, so `touch` rejected the value.
- **Approach:** resolve the requested revision once with the safe
  `rev-parse --verify --end-of-options <revision>^{commit}` pattern, then use
  that immutable commit for the version, epoch, and archive tree.
- **Regression:** an isolated annotated-tag fixture proves commit/tag/repeated
  archives are byte-identical and xz-valid, preserve the commit epoch, and
  reject option-like revisions without output.
- **Packaging boundary:** the regression runs as an explicit Ubuntu CI step
  with declared `git`, `python3`, and `xz-utils` tools. It is not registered in
  CTest, so Debian package tests gain no undeclared build dependencies.
- **Review:** complete three-file diff has no unrelated, obfuscated,
  credential, permission, dependency-source, or network behavior. Independent
  review found no blocker after the CI placement correction.
- **Commit:** `8c0ebe32de1f1aa55d489e2dadc24bf21fdce14c`, with `Refs #5`.
- **Status:** merged as `59d4f257` after all CI, Debian 13, dependency review,
  and CodeQL checks passed. Exact-main CI and CodeQL also pass.
- **Outcome:** `59d4f257` became the signed `v0.2.0` target. Tag-derived source
  and offline package reproduction passed before the pre-alpha was published.
