# Maintenance runs

## 2026-07-19

- Inbox: no unread notifications.
- Open issues: none.
- Open pull requests: PR #1 was assessed as low risk and merged.
- Dependabot alerts: none.
- Code scanning alerts: none; the initial CodeQL default-setup run passed for
  Actions, C/C++, and Rust.
- Repository safeguards: GitHub Actions are enabled with read-only default
  workflow permissions. Branch protection was deliberately not added because
  this is currently a single-developer repository.
- Artifact: `notes/pr-1.md`.
- Shipped locally: ABI v2 contact snapshot boundaries and authoritative Purple
  buddy-list create/update/delete, with focused reconciliation tests.
- System install: release build installed under `/usr/lib/x86_64-linux-gnu/purple-2`;
  the installed plugin passed the headless probe.
- Isolated-profile validation passed fresh QR linking, encrypted-store
  reconnect without a new QR, and direct messages in both directions.
- The first unsolicited snapshot contained one contact. An explicit contact
  request synchronized 46 contacts and projected all 46 as visible Purple
  buddies. Alias-update and stale-delete behavior remains test-only. Groups,
  typing, receipts, and second-device sync remain pending.
- User documentation now distinguishes queued offline delivery from unsupported
  historical import and records the Flare contact-sync comparison.

The installed maintainer package did not include its documented triage script
or reference files, so this run was captured manually and `state.json` was left
unchanged rather than guessing its schema.

## 2026-07-20

- Synchronized `main` with `origin/main` at `4b32662` (version 0.1.0). The
  release configuration intentionally skips GitHub releases, so there is no
  release object or tag newer than this revision.
- Compiled a clean release build and passed all CTest tests (3/3), Rust tests
  (15/15), and the installed headless module probe.
- Installed the plugin, private Rust backend, AppStream metadata, and protocol
  icons system-wide under `/usr`. Runtime dependency resolution and exact
  staged-versus-installed file comparisons passed.
- Pidgin had independently exited before installation. No process was stopped
  or restarted by this run; the next launch will load the new files.
- Inbox: no unread notifications.
- Open issues: ISSUE:3 is actionable and medium priority. Recommend adding
  README acknowledgements for tdlib-purple and Flare as architectural
  references, with an explicit clarification that this project is not a fork.
- Open pull requests: none.
- Dependabot alerts: none.
- Code scanning alerts: none.
- Artifact: `notes/issue-3.md`.
- Public actions: none; implementation, comments, labels, and issue state were
  left unchanged pending human approval.

The installed maintainer package still lacks its documented triage script and
reference files, so this run was captured manually and `state.json` was left
unchanged rather than guessing its schema.

## 2026-07-20 — post-README health refresh

- Repository head: `cb21753`; CI, CodeQL, and Release Please report successful
  workflow conclusions.
- Inbox, open issues, open pull requests, Dependabot alerts, and code-scanning
  alerts: none. Private vulnerability reporting, secret scanning, and push
  protection are enabled.
- ISSUE:3 is closed and was resolved by `3117bc0` with `Refs #3` in the commit
  body. No public maintenance action was taken in this run.
- Release automation needs attention despite its green conclusion. PR:2
  remains labelled `autorelease: pending`; Release Please logs that it aborts
  because an untagged merged release PR is outstanding.
- No tag or GitHub release exists. Do not tag `cb21753`: the reproducible
  `0.1.0-1` package and hardening evidence belong to `9f68633`/ABI v4, while
  current head is ABI v6 and still identifies as `0.1.0`.
- Highest product gate: freeze a new candidate version, rebuild and harden that
  exact revision, then complete controlled live group, attachment, delivery,
  identity, retry, upgrade, and failure-mode validation.
- Highest support-debt items: correct the private-backend install diagnostic,
  document source uninstall, add a build revision to both libraries, expand
  C/Rust boundary tests, and align Debian build instructions with the Rust 1.94
  requirement.

The installed maintainer package still lacks its referenced triage script and
reference files. GitHub state was gathered with `gh-helper` and `gh`; the local
state schema was not inferred or modified.

## 2026-07-21 — reconnect fix and pre-release preparation

- Fixed reconnect routing at `822414a`: group membership refresh now restores
  the friendly Purple title, and the backend classifies locally authored saved
  messages as outgoing and marks successful sends projected.
- Release, warning-as-error, Rust (40/40), Purple (4/4), focused reconnect, and
  GCC ASan/UBSan checks passed. The release build was installed system-wide and
  loaded by Pidgin from the expected plugin/private-backend paths.
- The live account projected 49 contacts and 10 groups without duplicate or
  missing entries. The reported group remained a single friendly chat after
  startup; idle Pidgin measured 0.000% CPU over a five-second sample, with no
  signal-purple warning/error lines. Exact queued-message replay still needs a
  fresh message delivered while the account is offline.
- Removed the stale release label from merged PR:2 and restored Release Please.
  Candidate PR:4 is open at `a0d80f8`, includes the Cargo lock version, and has
  green CI, dependency review, and CodeQL checks. It remains unmerged.
- Opened ISSUE:5 as the single 0.2.0 validation tracker. Source/package hashes,
  official-client matrix, exact offline replay, reproducibility, and soak gates
  remain open.
- Opened draft PR:6 for release/support documentation and a digest-pinned
  Debian 13 build/test/staged-install job. Primary CI, the Debian 13 job,
  dependency review, and CodeQL all passed; the PR remains unmerged.
- A tracking-upstream push sent the first PR:6 branch attempt to `main`.
  Normal revert commit `1bd2c90` restored the exact pre-push tree, and the
  hardening commits were recreated on the explicit feature ref. Main CI,
  Release Please, and CodeQL passed after the correction.
- GitHub's optional managed “Code scanning AI findings” job failed because its
  configured model was rejected as unsupported. Repository CodeQL and alert
  state are green; this is not a source or workflow failure.
- Current queue: ISSUE:5, PR:4, and draft PR:6. Dependabot and code-scanning
  alerts remain empty.

GitHub state was refreshed with `gh-helper` and `gh`. The maintainer package's
referenced optional files remain absent, so `state.json` was not inferred or
modified.

## 2026-07-21 — release queue review

- Inbox contains the review request for PR:4. The open queue is ISSUE:5, PR:4,
  and draft PR:6; Dependabot and code-scanning alerts are empty.
- Full diff and provenance reviews found no unexplained or malicious behavior.
  Project CI, dependency review, and CodeQL pass for both PRs. GitHub's optional
  AI scan fails because its selected model is unsupported, not because of a
  repository finding.
- PR:6 is technically sound but must land before candidate freeze because it
  changes packaging inputs and adds the required Debian 13 validation job.
- PR:4 is not ready: its body auto-closes ISSUE:5, its current candidate omits
  PR:6, and its release notes need an explicit cumulative-versus-deduplicated
  policy.
- ISSUE:5 and the release process disagree about whether the candidate is the
  PR head or the resulting merge commit. The recommended sequence is to merge a
  corrected PR:6, refresh and re-review PR:4, merge PR:4 without publishing,
  freeze the resulting `main` SHA, and validate/package/tag that exact commit.
- No public action was taken. Notes: `notes/issue-5.md`, `notes/pr-4.md`, and
  `notes/pr-6.md`.

The maintainer package's referenced triage script and reference files remain
absent. `state.json` was left unchanged rather than inferring its schema.

## 2026-07-21 — 0.2.0 candidate freeze

- Corrected PR:6 release sequencing in `0061ec4`, including the post-merge
  candidate definition and the rule that merging does not publish. The public
  PR body was aligned, all project checks passed, and PR:6 merged as `07cffdb`.
- Refreshed PR:4 with PR:6, curated cumulative first-public-release notes, and
  removed the stale auto-close relationship to ISSUE:5. The final six-file
  release diff passed independent review and all project checks at `66791a9`.
- PR:4 merged as `c14e355`; no tag or GitHub release was created. ISSUE:5 now
  records the full post-merge SHA as the frozen candidate and remains open.
- Exact-candidate main CI passed formatting, Clippy, Rust tests, CMake build,
  C tests, installed plugin probing, and the Debian 13
  build/test/staged-install job. All CodeQL analyses passed. The corresponding
  ISSUE:5 build gate is checked; 21 gates remain open.
- Release Please opened PR:7 for 0.3.0 because no 0.2.0 tag/release exists and
  it could not parse the manually curated merged PR body. PR:7 is not a valid
  next release candidate and was closed unmerged after explicit approval.
- Dependabot and code-scanning alerts were empty before execution. No source
  vulnerability finding was introduced.

The maintainer package's referenced triage script and reference files remain
absent. `state.json` was left unchanged rather than inferring its schema.

## 2026-07-21 — 0.2.0 automated candidate validation

- Candidate `c14e355` passed 40 Rust tests, 4 normal C/Purple tests, 4 GCC
  ASan/UBSan tests, and 9 tests from the exact pinned Presage store revision.
- Corrupt encrypted state, unavailable Secret Service, and runtime ENOSPC
  fault injection failed closed. Three corrected ENOSPC runs preserved the
  baseline outbox row, rolled back the failed write, reopened cleanly, and
  accepted a recovery write without exposing synthetic sensitive values.
- The vendored 0.2.0 source archive reproduced byte-for-byte. Two clean
  Debian 13 builds with networking disabled produced identical runtime and
  debug packages, then passed package tests, install, probe, private-backend
  resolution, and removal.
- A package lifecycle run passed 0.1 install, 0.2 upgrade, 0.1 rollback, 0.2
  re-upgrade, and uninstall with expected ABI transitions, one installation
  scope, and a preserved profile-state sentinel.
- A CMake source-install negative control reproduced timestamp-based rollback
  skipping. Manifest-first removal passed fresh install, upgrade, rollback,
  re-upgrade, and uninstall. Reviewed docs commit `071e89f` now requires that
  safe sequence; the change is documentation-only and does not replace the
  frozen runtime candidate.
- Final unsigned artifacts and `SHA256SUMS` are held outside the repository.
  A 390-package SPDX 2.3 snapshot was captured while `main` still exactly
  matched the candidate.
- A clean private 0.2.0 Pidgin profile was built, tested, staged, and left
  unlinked. The normal Pidgin process remains on 0.1.0 and was not changed.
- Exact-candidate live interoperability, network recovery, idle/diagnostic
  capture, and the minimum 24-hour soak require dedicated non-production
  accounts and an official-client operator. No GitHub issue edit, tag, or
  release was made.
- The intended feature-branch push followed its inherited `main` upstream
  because global `push.default=tracking` is configured, so `071e89f` landed
  directly on `main`. The intended reviewed docs change was retained rather
  than reverted. Release Please consequently opened invalid 0.3.0 PR:8; it
  remains unmerged pending explicit approval to close.

## 2026-07-21 — validation waiver and tracker update

- After explicit approval, invalid Release Please PR:8 was closed unmerged.
- ISSUE:5 was updated with full artifact hashes, exact-candidate evidence,
  honestly checked automated gates, an explicit signing gate, and the
  remaining live-dependent gates left unchecked.
- Dedicated non-production Signal accounts are unavailable. The release owner
  approved proceeding without the extra live test for this labelled pre-alpha;
  live interoperability, network recovery, idle/diagnostic capture, and soak
  remain unverified rather than passed.
- A permanent validation record and checklist update are being prepared on the
  local `maint/record-0.2-validation` branch. It is intentionally unpushed
  before tagging so Release Please cannot open another premature 0.3.0 PR.
- Publication remains blocked on a separate choice for the expired personal
  signing key and explicit approval to sign/tag/publish the pre-release.
- Independent review then found that tagging `c14e355` would ship the old
  source-rollback guidance. The candidate was refrozen locally at docs-only
  `071e89f`, which contains the reviewed manifest-first safety instructions.
- Exact-refreeze Rust (40/40), C/Purple (4/4), C-adapter ASan/UBSan (4/4),
  pinned Presage-store (9/9), staged-probe, and zero-alert checks passed.
- Two independent `071e89f` source archives matched at `7e0d8cb3…`; two
  network-disabled Debian 13 builds reproduced runtime `19190da4…` and dbgsym
  `ba982337…`. Those packages and the probe are byte-identical to the initial
  candidate, so the previously passed binary lifecycle evidence transfers.
- The updated SPDX snapshot is `07b9b470…` and unsigned `SHA256SUMS` is
  `524c20e8…`. A corrected ISSUE:5 body and permanent docs record commit
  `ba34f38` are prepared locally; no refreeze-related public action has yet
  been taken.

## 2026-07-21 — signing and annotated-tag release fix

- After explicit approval, personal GPG key
  `B3C0B75FA3B33AC278738C5CB1798BCDA76054BD` and its encryption subkey were
  renewed through 2028-07-21. The GitHub noreply UID was added so email privacy
  remains enabled, and GitHub registers both verified identities on the same
  signing-capable key.
- `SHA256SUMS` and a local `v0.2.0` tag were validly signed. GitHub initially
  reported `bad_email`; a corrected noreply-signed tag then verified as valid.
- Tag-based source reproduction exposed a release blocker before publication:
  the archive helper treated annotated-tag metadata and its signature as part
  of the commit timestamp. The remote tag was withdrawn; no release existed.
- PR:9 fixed the helper by peeling revisions to an immutable commit and added
  an isolated annotated-tag determinism/rejection regression in CI. Complete
  provenance and security review found no blocker. PR CI, Debian 13, dependency
  review, and CodeQL passed before merge.
- PR:9 merged as `59d4f257`; exact-main CI and CodeQL passed. This post-merge
  commit is the new candidate. Release Please opened invalid 0.3.0 PR:10 while
  the corrected 0.2.0 tag is absent; it must not merge.
- Two independent new-candidate source archives already match at `9e984421…`.
  Offline package rebuilds and final hashes are in progress before re-signing.

## 2026-07-21 — 0.2.0 pre-alpha publication

- Final candidate `59d4f257` reproduced from the plain signed-tag name: the
  source archive matched byte-for-byte, and a clean network-disabled Debian 13
  build passed package tests, install, probe, runpath/backend, and removal.
- Signed tag `v0.2.0` was pushed and GitHub verified its signature and exact
  target. The checksum manifest signature validates with the same approved
  fingerprint.
- Published the 0.2.0 GitHub pre-release with source, runtime, debug-symbol,
  SPDX, checksum, and signature assets. Downloaded copies of all six assets
  matched the verified local files; checksum and GPG verification passed.
- PR:11 recorded final validation evidence and retained every waived live gate
  as unchecked. PR CI, Debian 13, dependency review, and CodeQL passed; it
  merged as `478eac7f`, and resulting main CI, CodeQL, and Release Please runs
  also passed.
- ISSUE:5 was updated with final release evidence and closed with all four
  supported exit gates checked. Stale Release Please PR:10 was subsequently
  closed unmerged after separate explicit approval.
