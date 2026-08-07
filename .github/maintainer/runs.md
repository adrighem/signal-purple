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

## 2026-07-24 — 0.2.2 alpha publication

- Declared the project alpha quality in user-facing documentation and metadata,
  while retaining the explicit unofficial and unsupported-by-Signal warning.
- Released the local group-participant identity fix: the canonical ACI remains
  the participant identity, while Purple's local account alias takes precedence
  over the remotely fetched Signal profile name for display.
- Published signed prerelease tag `v0.2.2` at `34e9b672` with source, runtime,
  debug-symbol, SPDX, checksum, and detached-signature assets.
- Release-artifacts runs `30079094193` and `30080197066` passed reproducible
  source generation, two network-disabled Debian package builds, installation
  probing, normalized SBOM validation, provenance attestation, and safe
  draft/published asset matching.
- Downloaded release assets passed `SHA256SUMS`; the detached checksum signature
  verifies with the release signing key. Exact-candidate live Signal
  interoperability and soak testing remain explicitly waived for this alpha.

## 2026-07-25 — Dependabot queue review

- Repository `main` is clean at `6b63c70`; current main CI, CodeQL, and Release
  Please runs pass. Published pre-releases are v0.2.0, v0.2.1, and v0.2.2.
- Inbox and open queue contain PR:15 through PR:21. There are no open issues,
  Dependabot alerts, or code-scanning alerts.
- Complete diff and provenance review found PR:17 low-risk and merge-ready.
  PR:15 and PR:16 are also merge-ready with low-to-moderate residual risk
  because pull-request CI does not execute their release-only artifact paths.
  Recommended merge order is PR:17, PR:16, then PR:15, with checks revalidated
  after each merge.
- PR:18, PR:20, and PR:21 have green CI but must not merge as generated:
  Dependabot removed the release-sensitive signal-core version marker from
  `Cargo.lock`. PR:21 additionally needs an exact historical group-ID digest
  regression before changing the hash implementation dependency.
- PR:19 is not mergeable. Its breaking websocket upgrade duplicates the HTTP
  and websocket dependency stacks while pinned Presage remains on the old API;
  both normal and Debian builds fail. Defer it until the Signal stack can move
  in sync.
- PR CodeQL checks are neutral because the expected configurations were not
  found; repository CodeQL on current `main` passes. Notes: `notes/pr-15.md`
  through `notes/pr-21.md`.
- After explicit approval, PR:17 merged normally as `8c426259`. The installed
  GitHub credentials lack workflow scope, so API merges of PR:16 and PR:15
  were not viable. Their exact reviewed heads were merged through isolated
  worktrees and pushed as `f60b2d39` and `d62542c6`; GitHub recognized both
  source PRs as merged.
- Post-merge CI, Debian build/install/probe, CodeQL, and Release Please pass for
  all three commits. PR:15's first Release Please attempt failed on a GitHub
  GraphQL internal error while fetching merge history. A direct history query
  confirmed API recovery, and the failed job rerun passed without repository
  changes.
- The final inbox contains only PR:18 through PR:21. Dependabot and
  code-scanning alerts remain empty.
- PR:22 added a focused regression guard for the signal-core lockfile marker
  and Release Please generic extra-file mapping. It merged as `a5baaacf`; PR
  and exact-main validation passed.
- PR:18 was rebased onto the guard, its marker was restored, and the final
  futures-only diff merged as `a020a2a4`. PR:20 was then rebased onto that
  lockfile, repaired, and merged as `116156c9`.
- PR:21 was rebased onto the combined graph, gained an independently reproduced
  historical group-ID digest vector, restored the marker, and merged as
  `6160293a`. The final graph intentionally retains transitive sha2 0.10.9
  alongside direct sha2 0.11.0.
- The rebased Dependabot commits and maintainer repair commits for PR:18,
  PR:20, and PR:21 are unsigned. Their account provenance and complete diffs
  were independently reviewed and fully explained; do not describe them as
  cryptographically verified.
- PR:19 received the approved compatibility explanation and was closed
  unmerged. Revisit reqwest-websocket 0.6 only with a coordinated
  Presage/libsignal-service upgrade and live reconnect validation.
- Final exact-main CI, Debian build/install/probe, CodeQL, and Release Please
  passed after each dependency merge. No source or alert failure was retried
  without diagnosis.

## 2026-07-25 — Release Please architecture migration

- Commit `6efa21e` made Release Please the owner of release tags and draft
  GitHub releases, connected its exact outputs directly to the reusable
  artifact workflow, and removed the manual tag, dispatch, and
  published-release recovery paths.
- Release publication now remains private through reproducible builds,
  package probing, SBOM and checksum generation, provenance attestation, and
  digest-verified uploads. The final job publishes a non-Latest prerelease.
- Focused release tests, artifact-helper tests, YAML parsing, upstream config
  schema checks, actionlint 1.7.12, and an independent architecture/security
  review passed before push.
- Release Please run `30158948607` initially failed because historical release
  PR:4 and PR:13 still had `autorelease: pending`, causing the newly enabled
  releaser to replay the former manual-tag path. After explicit approval, the
  stale labels were replaced with `autorelease: tagged`; the failed job rerun
  passed and correctly skipped artifacts for the non-release-bearing `ci:`
  commit.
- Exact-main CI run `30158948498` and CodeQL run `30158948254` passed. No tag,
  draft, or release was created during the migration.

The installed maintainer package still lacks its referenced triage script and
reference files, so this run was captured manually and `state.json` was left
unchanged rather than inferring its schema.

## 2026-07-25 — Architecture and contributor-maintainability refactor

- Parallel production, Rust-test, and contributor-workflow reviews identified
  event delivery, stale CMake Rust dependencies, and validation-command drift
  as the highest-leverage behavior-preserving refactor seams.
- Event producer/consumer state, descriptor notification, overflow, and byte
  accounting moved into one module. Boundary tests now use small injected byte
  budgets and still observe the real Unix-stream notification token.
- Attachment-task panics retain their request identity and become bounded
  request failures instead of leaving a transfer pending.
- Cargo now remains the Rust dependency authority during every CMake build.
  A repeated no-change CMake build invoked Cargo and completed incrementally.
- `scripts/check.sh` became the shared contributor and primary-CI gate, with
  warning-as-error policy and a reusable staged-install probe.
- Rust formatting, 51 Rust tests, release-helper tests, actionlint, yamllint,
  shell syntax checks, CMake/Ninja with `-Werror`, four CTest targets, and the
  staged plugin installation/load probe passed locally. Local Clippy awaits CI
  because the host has Clippy 1.87 beside Rust/Cargo 1.95.
- The remaining attachment-admission, recovery-state, connection-lifecycle,
  projection-acknowledgment, and ABI-conformance work is recorded in
  `work/architecture-refactor-2026-07-25.md`.

## 2026-07-30 — v0.3.0 health and empty-queue audit

- `main`, `origin/main`, and prerelease tag `v0.3.0` all identify
  `be4b9a8`. The worktree was clean before this maintenance run.
- The repository inbox, open issue and pull-request queues, Dependabot alerts,
  code-scanning alerts, and private security-advisory queue are empty. Current
  main CI, Release Please, dependency review, and scheduled CodeQL pass.
- All four downloaded v0.3.0 payloads pass the published `SHA256SUMS`.
  GitHub's attestation API reports one SLSA v1 bundle binding those four names
  and digests to release workflow run `30200070988` and commit `be4b9a8`.
  The installed GitHub CLI cannot independently verify the Sigstore bundle,
  and `SHA256SUMS` itself is not an attested subject.
- The recorded release-trust design depends on protected `main`, but the audit
  initially found no branch protection or rulesets. With approval, active
  rulesets now require pull requests, current CI, dependency review, CodeQL,
  code-scanning results, and block deletion and force-push of `main`. Layered
  tag rules let only Release Please create `v*` tags while allowing nobody to
  update or delete them. Future releases are immutable. The existing v0.3.0
  release record predates that setting and remains mutable, while its tag
  reference is now protected.
- Local Rust formatting, all 87 signal-purple Rust tests, all 35 Presage tests,
  all 11 Presage SQLite-store tests, release-helper checks, the warning-as-error
  C build, all five C/Purple tests, and the staged install probe pass. The full
  Presage workspace also checks with all targets. Local Clippy remains
  unavailable because the installed component is 1.87 while rustc and current
  dependencies require newer versions; exact-main CI passed Clippy with its
  matched toolchain.
- Static review found that incoming attachment size checks run after Presage
  has materialized the complete download, so missing or understated metadata
  can exceed the intended allocation boundary. A bounded upstream download
  API is required. Pending read receipts can also grow with every unread
  message in an unfocused conversation and need an explicit bounded policy.
- A local UUID-only buddy confirmed the generic profile-fallback gap: an empty
  synchronized contact name and phone number leave Purple with no server alias,
  while the baseline Presage contact sync can erase and then suppress repair of
  a profile-derived fallback. No personal identifier was recorded here.
- Prepared local CI hardening so the supported Debian 13 job runs Rust tests,
  explicitly enables C tests, and fails when CTest discovers zero tests.
  Removed the duplicate released SQLite fix from the `Unreleased` changelog.
- Prepared an unpushed Presage commit that bounds attachment downloads before
  allocation, validates encrypted framing before decryption, preserves
  canonical profile keys during contact sync, and repairs empty contact names
  from cached or freshly fetched Signal profiles. Profile refreshes are
  sequence-ordered and coalesced by contact, use a bounded background fetch,
  serialize final merges with contact sync, and drain before Presage reports an
  empty receive queue. The local signal-purple branch pins that commit and uses
  the remaining per-message byte budget as the download limit.
- Approved repository-setting changes were applied and verified. After separate
  publication approval, the Presage dependency branch was pushed at `82c215e`
  and PR:26 merged the reviewed signal-purple branch as `a8f383a`. No issue,
  comment, or release edit was made.

The installed maintainer package still lacks its referenced triage script and
reference files. `state.json` remains unchanged rather than inferring its
schema.

## 2026-07-31 — Incoming animation presentation

- A newly received group attachment with no remote filename was confirmed as a
  short, square H.264 MP4 without audio. This is compatible with a Signal GIF
  attachment, but the stored file alone cannot prove its Signal attachment
  flag.
- Pidgin 2.14.14's conversation image path animates GdkPixbuf-supported image
  data but has no inline representation for MP4 video. Automatic transcoding
  would add an untrusted-media decoder, CPU and memory pressure, and a new
  runtime dependency; a Pidgin-specific video widget would break the protocol
  plugin's Purple UI boundary.
- Prepared genuine GIF inline routing with pre-decode structure, compressed
  size, dimension, and cumulative frame-area validation. Unnamed JPEG, PNG,
  GIF, and MP4 attachments receive usable fallback filenames, including the
  Signal GIF flag when present.
- Rust formatting, matched-toolchain Clippy, all 88 Rust tests, release-helper
  tests, the warning-as-error C build, all five C/Purple tests, and the staged
  installation probe pass locally. Live validation remains required for a real
  GIF payload and the inferred MP4 filename.

## 2026-07-31 - Bounded Signal GIF-style MP4 conversion

- Approved and implemented automatic conversion only for incoming group MP4s
  with Signal's GIF flag, an exact `video/mp4` type, a valid first `ftyp` box,
  and an input no larger than 8 MiB. Ordinary and direct video is unchanged.
- The optional Debian FFmpeg child runs through `prlimit` with fixed absolute
  paths and arguments, a cleared environment, pipe-only protocols, one global
  process, two attempts per message, one codec/filter thread, 512 MiB address
  space, 10/12 seconds CPU, 15 seconds wall time, 64 descriptors, a 480-pixel
  edge and 15-fps ceiling, and an 8 MiB output ceiling. No media file is made.
- The original MP4 stays owned by the receive projection until generated GIF
  structure, cumulative frame area, and the 50 MiB presentation budget pass;
  missing tools, contention, process or I/O failure, timeout, and invalid or
  over-budget output retain the original Purple receive prompt.
- The exact reported 480x480, 1.87-second sample converts to 28 GIF frames under
  the final policy. A production-path synthetic MP4 conversion test passes, as
  do matched Rust 1.95 formatting and Clippy, all 93 Rust tests, release-helper
  checks, the warning-as-error C build, all five C/Purple tests, and the staged
  install/plugin-load probe. Live receipt of a newly flagged Signal GIF remains
  pending.

## 2026-07-31 - Startup contact-sync pool timeout

- A live restart reported that the explicit contact-sync request timed out
  waiting for Presage's SQLite pool. The pinned store intentionally permits one
  connection, while the receive stream performs registration and pre-key work
  before its first `QueueEmpty`; starting contact sync at stream creation let
  those phases overlap on the sole connection.
- Contact sync now waits behind an explicit gate until the first drained-queue
  path has completed its local projections and declared the account ready. Its
  existing bounded retries and shutdown abort remain unchanged, and the pool is
  not widened.
- Added a regression test proving the gated operation is not polled early.
  Documented Purple's curated `signal-purple` debug surface and the deliberate
  exclusion of raw Presage tracing, which can contain private metadata.
- Matched Rust 1.95 formatting and warning-as-error Clippy, all 94 Rust tests,
  release-helper checks, the warning-as-error C build, all five C/Purple tests,
  and the staged install/plugin-load probe pass. One initial full run reported
  all 94 tests successful and then aborted in glibc allocator teardown; the same
  binary passed one direct rerun, ten consecutive stress reruns, and the clean
  full rerun. Live restart validation of the scheduling fix remains pending.

## 2026-07-31 - Receive-stream store starvation

- Live validation of v0.4.1 started normally, then reported pool-acquisition
  timeouts while reading the encrypted outbox and acknowledging displayed
  messages. No second signal-purple process remained during investigation.
- The actor polled Presage's long-lived `unfold` receive stream directly in the
  same `select!` as acknowledgements, commands, and retry timers. If another
  branch won while the stream retained a pending future that owned the sole
  SQLite connection, the actor awaited that connection without continuing to
  poll the future which could release it.
- The receive stream now runs in an independently scheduled task on the same
  `LocalSet` and forwards ordered values through a bounded 16-item channel. The
  actor still owns all projection and command state. Receive, contact-sync, and
  attachment tasks are aborted and joined together at active-generation
  shutdown boundaries.
- A regression test models the receive future retaining an exclusive store
  slot while the actor waits and proves the independent forwarder releases it.
  Matched Rust 1.95 formatting and warning-as-error Clippy, all 95 Rust tests,
  release-helper checks, the warning-as-error C build, all five C/Purple tests,
  and the staged install/plugin-load probe pass.
- An initial unlocked focused Cargo invocation normalized away the
  release-please marker comment in `Cargo.lock`; the tracked marker was restored
  with no dependency change, and the clean locked full gate passed. Live
  restart validation of the receive-driver fix remained pending at merge time.
- The immutable `v0.4.2` package was subsequently installed and started against
  the existing encrypted store. Continued use reproduced none of the prior
  contact-sync, outbox-read, or displayed-message acknowledgement pool
  timeouts. This closes the focused live validation gap, but is not a
  multi-account or long-duration soak test.
- A scoped read-only Purple query also confirmed that the previously reported
  UUID-only buddy now has a nonempty local/effective alias despite an empty
  server alias, with no open conversation retaining a stale title. No personal
  identifier or alias text is recorded in repository memory.

## 2026-07-31 - Loaded-module probe symbol isolation

- The headless plugin probe links buddy-list, contact-sync, and group-sync
  helpers into an export-dynamic executable while loading a module containing
  the same global symbols. The module's dynamic helper relocations therefore
  allowed the executable copies to preempt the packaged implementations,
  weakening callback coverage despite a successful module load.
- The plugin now binds its internal function references locally. A new CTest
  relocation gate fails if any duplicated sync helper remains dynamically
  interposable, while the existing helper-focused tests and loaded-module probe
  remain intact. Direct, staged-install, and release-package probes now have
  hard execution timeouts.
- Matched Rust 1.95 formatting and warning-as-error Clippy, all 95 Rust tests,
  release-helper checks, the warning-as-error C build, all six C/Purple tests,
  and the staged install/plugin-load probe pass locally.
- The first staged probe exhausted the shared `/tmp` tmpfs while copying the
  debug backend. Three confirmed unused signal-purple build/review directories
  from this maintenance work were removed, reclaiming 2.6 GiB; the unchanged
  probe then passed. No repository or live Pidgin state was removed.

## 2026-07-31 - Projection acceptance controls acknowledgement

- The C event dispatcher previously returned only whether its GLib source
  should continue. The poller treated that result as proof that every direct,
  group, or attachment projection was accepted, so a handler which rejected
  malformed required fields could still release the durable encrypted message.
- Dispatch now reports projection acceptance separately. Missing or empty
  routing identifiers, absent message text, inconsistent attachment routing,
  and absent attachment bytes keep the event source alive but leave the
  projection unacknowledged for replay. Explicit size, resource, and
  presentation-policy rejections remain terminal after user notification.
- The loaded-module probe exercises malformed direct, group, and attachment
  events plus valid direct and group controls. Matched Rust 1.95 formatting and
  warning-as-error Clippy, all 95 Rust tests, release-helper checks, the
  warning-as-error C build, all six C/Purple tests, and the staged
  install/plugin-load probe pass.
- A C-only GCC AddressSanitizer/UndefinedBehaviorSanitizer build also passed all
  six tests with LeakSanitizer disabled for libpurple's process-global
  registries. Clang's sanitizer runtime was not installed; an initial GCC run
  exported sanitizer `CFLAGS` into Cargo and was discarded after restoring the
  normal Rust target. Passing sanitizer flags through CMake kept the final run
  scoped to C as intended.
- PR #38 and its six-file Release Please follow-up merged as immutable
  prerelease `v0.4.4`. The exact tag, five asset digests, four-subject SLSA
  provenance, and commit-plus-assets release attestation were verified before
  installing `signal-purple 0.4.4-1`; `dpkg --verify` passed. No Pidgin process
  was restarted for installation.

## 2026-07-31 - Deterministic session transition model

- The backend actor independently mutated recovery, synchronization,
  group-authority, and group-dirtiness booleans. Their valid combinations were
  implicit across nested receive-start and active-session loops, making later
  module extraction likely to change gates accidentally.
- A single session model now owns `Initializing`, `Recovering`, and `Ready`
  phases, bounded recovery state, and `Pending`, `Authoritative`, or `Dirty`
  group snapshots. Receive failure revokes group authority, successful initial
  queue processing alone marks the core ready and resets backoff, and only
  authoritative group content can dirty a snapshot.
- A focused transition test covers initial readiness, snapshot invalidation,
  recovery entry versus continuation, last-error replacement, retry progression,
  and backoff reset. Matched Rust 1.95 formatting and warning-as-error Clippy,
  all 96 Rust tests, release-helper checks, the warning-as-error C build, all six
  C/Purple tests, and the staged install/plugin-load probe pass locally.

## 2026-07-31 - SignalConnection owner lifecycle

- Login, startup failure, the loaded-module probe, and normal close previously
  maintained separate copies of `SignalConnection` initialization or cleanup.
  Adding an owned field could therefore leave a test fixture unrepresentative or
  one failure path incomplete.
- One production constructor now initializes every adapter-owned field, and one
  finalizer releases the notifier, core, containers, sync state, and strings.
  Startup failure and normal close share the finalizer; Purple protocol data,
  signals, requests, pending callback back-references, and transfers remain
  detached first on the normal close path.
- The loaded-module probe now resolves and uses the production constructor,
  observes owned `GSource` finalization, and verifies protocol-data detachment
  plus the closing marker from an active transfer cancellation callback.
  Matched Rust 1.95 formatting and warning-as-error Clippy, all 96 Rust tests,
  release-helper checks, the warning-as-error C build, all six C/Purple tests,
  and the staged install/plugin-load probe pass locally. A C-only GCC
  AddressSanitizer/UndefinedBehaviorSanitizer run also passed all six tests with
  LeakSanitizer disabled for libpurple's process-global registries.
- PR #41 merged as `1e8256b`; its post-merge CI, Debian 13 installation job,
  CodeQL aggregate, and Release Please run passed. The refactor correctly
  produced no release, and its isolated build artifacts were removed.

## 2026-07-31 - Compiled ABI conformance

- The existing C-only and Rust-only ABI assertions locked readable values but
  could not detect disagreement between the two independently maintained
  declarations.
- The public header now declares all FFI input bounds and an indexed diagnostic
  contract. The production Rust library reports 64 compiled values covering
  ABI version, statuses, events, flags, both public struct sizes, alignments and
  field offsets, and nine input limits.
- The existing C utility test links the real backend and compares every value
  against its independently compiled header representation, including the
  out-of-range sentinel. Matched Rust 1.95 formatting and warning-as-error
  Clippy, all 96 Rust tests, release-helper checks, the warning-as-error C build,
  all six C/Purple tests, and the staged install/plugin-load probe pass locally.
- PR #42 merged as `8ff1aaf`; its post-merge CI, Debian 13 installation job,
  CodeQL aggregate, and Release Please run passed. The test-only change
  correctly produced no release, and its isolated build artifacts were removed.

## 2026-07-31 - Installation lifecycle and pinned C formatting

- User upgrade, rollback, relink, installed-file removal, and account-data
  removal now live together in the README. The release-process document retains
  only maintainer candidate validation, trust, automation, publication, and
  rollback-readiness responsibilities.
- The canonical fast and full gates require clang-format major version 19 and
  scan every C source and header without relying on Git metadata. Check and fix
  modes share one inline style which preserves intentional line wrapping.
- Debian 13 clang-format 19.1.7 and Ubuntu 24.04 clang-format 19.1.1 produce the
  same normalized tree. The gate also rejects a missing formatter and a wrong
  major; shell syntax, gate assertions, and the GitHub workflow pass actionlint.
- Matched Rust 1.95 formatting and warning-as-error Clippy, all 96 Rust tests,
  release-helper checks, the warning-as-error C build, all six C/Purple tests,
  and the staged install/plugin-load probe pass locally with the new format gate.

## 2026-08-07 - Scoped stable declaration

- Declared the maintained 1.x line stable for the documented Debian 13 and
  Ubuntu 24.04 LTS packages. Fedora RPM and Nix remain best-effort build
  outputs, and the feature exclusions and live-validation dates remain explicit.
- Replaced stale pre-alpha and future-1.0 guidance in active documentation,
  recorded that v1.1.0 was the first published stable release, aligned
  Conductor context, and made the release checklist reusable for stable
  maintenance. Historical records remain unchanged.
- Changed the event queue from fatal aggregate overflow to bounded producer
  backpressure with shutdown wakeup. Startup replay now fetches the unprojected
  set once per connection, admits at most 64 projections concurrently, and
  applies receive-side backpressure after 64 deferred live messages.
  Delivery- and read-receipt metadata are deduplicated and capped at 4096;
  read receipts retry synchronous queue pressure and readiness transitions.
- Installed all project and third-party license texts in staged packages,
  aligned the Nix version and dual-license metadata, pinned cache actions by
  commit, required the FFmpeg path in both supported CI jobs, and verified the
  recorded Presage revision across dependency documents. Shipped AppStream
  metadata now carries the same scoped stable declaration and passes both XML
  and offline AppStream validation.
- The sanitized full gate passed clang-format 19, Rust formatting,
  warning-as-error Clippy, all 105 Rust tests including required FFmpeg
  conversion, release and APT helpers, the warning-as-error C build, all six
  C/Purple tests, and the staged install/plugin-load probe. A C-only GCC
  AddressSanitizer/UndefinedBehaviorSanitizer build also passed all six tests
  with LeakSanitizer disabled for libpurple's process-global registries.
- Public GitHub inspection found no open issues, pull requests, Dependabot,
  code-scanning, or secret-scanning alerts. The latest v1.2.0 release is
  published as the non-prerelease Latest release with its expected assets.
- Residual limit: Presage still materializes all unprojected rows in one query,
  so unusually large offline backlogs can raise peak startup memory despite the
  bounded projection window. No new live Signal 1.x validation was performed.
- PR #60's first two Debian 13 runs failed the FFmpeg converter probe inside the
  parallel Rust harness, while the exact command and isolated Rust test passed
  in the workflow's pinned Debian image. The canonical required-FFmpeg gate now
  runs that resource-limited process probe separately and both supported CI jobs
  use the canonical gate.
  The user approved the commit, branch push, and PR #60 after repository rules
  rejected a direct `main` push as designed. Release creation remains owned by
  Release Please; no tag, release, or public comment is created manually.
