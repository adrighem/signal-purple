# Issue #5: validate the 0.2.0 pre-release candidate

- **Intent:** collect sanitized evidence for one exact revision before signing,
  tagging, packaging, and publishing 0.2.0. Completed 2026-07-21.
- **Priority:** completed release gate; issue closed.
- **Released candidate:** `59d4f257a3b2514261d3fc773da4e9df90d9ffd4`,
  the PR #9 merge commit containing the reviewed annotated-tag archive fix.
- **Completed evidence:** exact-candidate main CI, Debian 13 staged install,
  CodeQL, 40 Rust tests, 4 C/Purple tests, GCC ASan/UBSan, and 9 pinned
  Presage-store tests pass. Corrupt-store, unavailable-keyring, and real
  runtime-ENOSPC fault injection fail closed and recover without exposing
  synthetic sensitive values.
- **Packaging:** the vendored source archive is deterministic. Two clean,
  network-disabled Debian 13 builds produce byte-identical runtime and debug
  packages. Fresh install, 0.1→0.2 upgrade, rollback, re-upgrade, and removal
  pass with one installation scope and preserved profile-state sentinel.
- **Source installation:** a negative control proved that installing an older
  CMake build over newer files can be skipped by timestamp checks. The safe
  manifest-first replacement sequence passes and is included in the candidate.
- **Artifacts:** source archive `9e984421…`, runtime package `19190da4…`,
  debug package `ba982337…`, and SPDX document `c2ecc09e…` are published with
  `SHA256SUMS` `7092498a…` and its detached signature `ffda6563…`.
- **Live-validation waiver:** dedicated non-production Signal accounts are not
  available. The release owner explicitly accepted proceeding without the
  extra live test for this pre-alpha. Exact-candidate interoperability,
  network recovery, idle/diagnostic capture, and soak remain unchecked and
  unverified; no official Signal client version was tested.
- **Publication:** signed annotated tag `v0.2.0` targets the released candidate
  and GitHub verifies it with fingerprint
  `B3C0B75FA3B33AC278738C5CB1798BCDA76054BD`. The pre-alpha release is
  published with six independently downloaded and verified assets.
- **Automation note:** invalid Release Please PRs #7 and #8 were closed
  unmerged after explicit approval. Stale 0.3.0 PR #10 was also closed
  unmerged after publication. Validation-record PR #11 merged as `478eac7f`,
  with all PR and resulting `main` checks green.
- **Refreeze:** the original candidate was refrozen first at the safe rollback
  docs commit and finally at `59d4f257` after tag reproduction exposed and PR
  #9 fixed the annotated-tag archive defect. Final archive and offline package
  reproduction passed from the signed tag.
- **Recommendation:** retain the pre-alpha compatibility warning and run the
  waived live/network/idle/diagnostic/soak exercises later with dedicated
  non-production accounts.
- **Confidence:** high on automated evidence; live compatibility remains
  unverified for the exact candidate.
- **Public action:** after explicit approval, the signed pre-release was
  published, PR #11 merged the permanent evidence, and ISSUE #5 was updated
  with final hashes and all supported exit gates before closing as completed.
