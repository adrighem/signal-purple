# Release process

This is the maintainer release and publication procedure. User-facing install,
upgrade, rollback, relink, and removal guidance lives in the README's
[installation lifecycle](../README.md#installation-lifecycle).

1. Confirm the roadmap scope and identify the applicable release-checklist
   gates.
2. Land every intended runtime, storage, dependency, packaging, CI, and release
   hardening change before freezing the candidate.
3. Open one candidate-validation issue to record the environment, evidence,
   and blockers. Leave its candidate revision pending at this stage.
4. Let release-please create or update the version and changelog pull request.
5. Review every dependency and generated-file change, then merge the release
   pull request only after every applicable release gate and its checks pass.
   Merging this pull request is the release approval.
6. Release Please creates the canonical `vMAJOR.MINOR.PATCH` tag and a draft
   GitHub release for the merged `main` commit.
7. In the same workflow graph, the artifact pipeline verifies the Release
   Please tag, commit, version, and manifest. It then reproduces the source
   archive and Debian packages twice, installs and probes the package, creates
   the SBOM and checksums, and attests their provenance.
8. The final job uploads the verified assets and publishes the draft as a
   GitHub prerelease without marking it as `Latest`. A failed build leaves the
   release private and does not move or recreate its tag.

The workflow uses a repository-scoped installation token from the private
Release Please GitHub App. The App has only Contents and Pull requests
read/write permissions and is installed only on this repository. This lets
release pull requests trigger their checks automatically; events made with the
repository `GITHUB_TOKEN` do not start new workflow runs. The workflow fails
closed if its App Client ID variable or private-key secret is unavailable.

The artifact workflow is reusable only from the trusted Release Please
workflow; it has no public event or manual dispatch trigger. Release Please
passes the exact tag, version, and 40-character commit. The caller must be a
`main` push at that commit, the tag must resolve to it, the commit must remain
on `main`, and the release manifest, Cargo files, citation metadata, and
`version.txt` must agree.

Build, attestation, and publication use separate jobs. Only the final job can
write repository contents, and it does not check out or execute repository
code. Existing assets are skipped only when their GitHub-reported SHA-256
digest agrees; a conflicting asset fails the run and is never overwritten.
Payloads are uploaded before `SHA256SUMS`, making an interrupted run safely
resumable by rerunning failed jobs in the original Release Please run.

Release tags are created by Release Please rather than signed by a maintainer.
Source trust comes from the protected `main` history, the checksum-pinned
Release Please action, exact tag/commit/version checks, least-privilege job
permissions, and GitHub's OIDC-backed artifact attestations. The public key in
`keys/release-signing-key.asc` remains only for verification of historical
releases.

Do not publish a release from a working tree with only compilation evidence.

## Candidate validation tracker

Use one issue as the evidence index for each release candidate. The 0.2.0
pre-release candidate is tracked in
[issue #5](https://github.com/adrighem/signal-purple/issues/5).
Record the release pull-request revision, Debian image or environment, official
Signal client versions and test date, artifact hashes, and links to sanitized
evidence. Keep the issue open through packaging and publication; a release
pull request must not close it automatically.

Evidence counts only for the recorded release pull-request tree. If runtime,
storage, dependency, or packaging inputs change, update the candidate revision
and rerun the affected checks. Release Please builds and identifies artifacts
from the resulting `main` commit after merge. Use dedicated non-production
Signal accounts, keep failed checks open, and link implementation defects
instead of creating separate validation trackers. Never attach identifiers,
message contents, provisioning data, keys, database secrets, or unredacted
private paths.

## Rollback readiness

The release owner decides to roll back when an upgrade cannot load, reconnect,
or preserve the buddy/group projection, or when a security or message-delivery
regression is found. Release artifacts and the previous package must remain
available until the soak period ends. Use the user-facing
[rollback procedure](../README.md#rollback) for candidate lifecycle checks.
