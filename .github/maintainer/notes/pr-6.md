# PR #6: harden pre-release validation

- **Intent:** add a Debian 13 build/test/staged-install job, complete Debian
  build dependencies, and tighten release, support, troubleshooting, removal,
  issue-reporting, and privacy guidance.
- **Provenance:** same-repository owner branch with two linear, unsigned commits
  based directly on `main`. Their tree matches the earlier owner-authored
  hardening changes that were accidentally pushed and then reverted.
- **Scope:** ten text files, 267 additions and 50 deletions. No runtime source,
  Cargo lock, generated artifacts, binaries, file modes, secrets, or elevated
  workflow permissions change.
- **Supply chain:** workflow permissions remain read-only; checkout is
  SHA-pinned with credentials disabled; the official Debian image is
  digest-pinned; and the Rust archive checksum matches the official Rust
  checksum. Networked apt and Cargo inputs remain mutable, which the docs
  correctly distinguish from reproducible offline release evidence.
- **Validation:** both project CI jobs, 4/4 Debian CTest tests, staged module
  loading, dependency review, CodeQL, YAML parsing, and whitespace checks pass
  at `357d911`.
- **Blocker:** the PR body/process currently treats `a0d80f8` as a frozen
  candidate while this PR changes a packaging input and supplies a required
  validation job. Its Debian pass therefore cannot count for that candidate.
- **Recommendation:** keep the PR draft until release sequencing says to land
  this hardening first, refresh and merge PR #4, freeze its resulting `main`
  SHA, and validate that exact revision. After that correction, PR #6 is safe
  and maintainable to merge.
- **Confidence:** high.
- **Public action:** none; edits, readying, or merge require human approval.
- **Outcome:** release sequencing was clarified in `0061ec4`, the public body
  was aligned, all refreshed checks passed, and PR #6 was merged as
  `07cffdb3308c5f632365b145011657a66459d89b` after approval.
