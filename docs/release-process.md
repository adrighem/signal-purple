# Release process

This is the maintainer release and publication procedure. User-facing install,
upgrade, rollback, relink, and removal guidance lives in the README's
[installation lifecycle](../README.md#installation-lifecycle).

1. Confirm the supported scope and identify the applicable stable-release
   requirements.
2. Land every intended runtime, storage, dependency, packaging, CI, and release
   hardening change before freezing the candidate.
3. Open one candidate-validation issue to record the environment, evidence,
   and blockers. Leave its candidate revision pending at this stage.
4. Let release-please create or update the version and changelog pull request.
5. Review every dependency and generated-file change, then merge the release
   pull request only after every applicable release requirement passes.
   Merging this pull request is the release approval.
6. Release Please creates the canonical `vMAJOR.MINOR.PATCH` tag and a draft
   GitHub release for the merged `main` commit.
7. In the same workflow graph, the artifact pipeline verifies the Release
   Please tag, commit, version, and manifest. It then reproduces the source
   archive, Debian 13 and Ubuntu 24.04 LTS packages, and best-effort Fedora RPM
   package twice, installs and probes both supported distro packages, creates
   the SBOM and checksums, and attests their provenance.
8. The final artifact job uploads the verified assets and publishes the draft
   as a stable GitHub release marked `Latest`. A failed artifact build deletes
   only the still-private draft and its matching tag.
9. The release job dispatches a top-level APT repository workflow, receives its
   exact run ID, and waits for completion. The child rebuilds the signed
   repository from the highest two stable semantic versions and deploys its
   Debian 13 and Ubuntu 24.04 suites through GitHub Pages. If deployment fails,
   manually dispatch the APT workflow without inputs; it never changes a
   release, tag, or asset.

The workflow uses a repository-scoped installation token from the private
Release Please GitHub App. The App has only Contents and Pull requests
read/write permissions and is installed only on this repository. This lets
release pull requests trigger their checks automatically; events made with the
repository `GITHUB_TOKEN` do not normally start new workflow runs. The APT job
uses GitHub's explicit `workflow_dispatch` exception with only Actions write
permission, then polls the exact returned run through the Actions API. The
workflow fails closed if its App Client ID variable or private-key secret is
unavailable.

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

APT metadata uses a separate signing key held only as the
`APT_SIGNING_PRIVATE_KEY` secret in the protected `apt-repository` environment.
Its passphrase is the `APT_SIGNING_KEY_PASSPHRASE` secret in the same
environment. The non-secret `APT_SIGNING_KEY_FINGERPRINT` environment variable
must match its uppercase primary-key fingerprint. The workflow exports only the
public key into the Pages artifact. Enable Pages with GitHub Actions as its
source before the first deployment. The `apt-repository` and `github-pages`
environments must allow the `main` branch. Both release-driven and manual runs
use that protected branch. APT publication is a top-level workflow run so those
environment secrets resolve only inside its signing job. Automated dispatches
bind the newest stable release and Git tag to the exact Release Please tag and
commit; manual repair runs omit that expected-release pair. The child checks out
the trusted commit selected when its `main` dispatch was created.

Cancelling the parent after dispatch does not cancel the child APT run. The
parent's 60-minute limit currently exceeds the child's 40-minute job budget and
neither protected environment has an approval wait. Revisit cancellation or
timeout handling if those assumptions change.

Do not publish a release from a working tree with only compilation evidence.

## Candidate validation tracker

Use one issue as the evidence index for each release candidate and link it from
the release pull request. Record the release pull-request revision, build images
or environments, Signal client versions and test date, artifact hashes, and
links to sanitized evidence. Keep the issue open through packaging and
publication; a release pull request must not close it automatically.

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
