# Release process

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

## Upgrade

1. Disable the Signal account and close Pidgin so the encrypted database is
   quiescent.
2. Keep a copy of the database path shown in the account's advanced settings.
   The matching secret-service entry is labelled `signal-purple database for
   <account>` and is required to open that copy.
3. Install the complete new package or follow the source-install replacement
   procedure below. Never mix a plugin from one revision with a backend library
   from another.
4. Start Pidgin, enable the account, and confirm it reconnects without a QR,
   then confirm contacts, groups, and a direct send/receive round trip.

Store migrations are automatic and additive. Keep the pre-upgrade database and
secret until the new version has completed its validation period.

For a CMake source install, replace revisions instead of installing one over
the other. Inspect the `install_manifest.txt` saved from the currently
installed build, verify that every entry belongs to the active installation
prefix, and remove exactly those files. Then run `cmake --install` from the
complete target build. CMake can otherwise report a target artifact as
`Up-to-date` when the installed file has the same or a newer timestamp,
potentially leaving the plugin and backend on different revisions. Do not
remove a plugin directory recursively.

## Rollback

Close Pidgin and reinstall the previous complete package. For a CMake source
rollback, first remove exactly the files in the currently installed build's
saved manifest as described above, then install the complete previous build.
Do not run an older `cmake --install` over newer files or replace one shared
library independently. Restore the matching pre-upgrade database only if the
older version cannot open the upgraded copy. If a rollback cannot reconnect,
return to the new build and its database rather than deleting state. Relinking
is the last recovery option because it creates a new Signal linked device.

The release owner decides to roll back when an upgrade cannot load, reconnect,
or preserve the buddy/group projection, or when a security or message-delivery
regression is found. Release artifacts and the previous package must remain
available until the soak period ends.

## Relink and removal

To relink without destroying recoverable state, disable the account and choose
a new empty encrypted-store path in its advanced settings. Re-enable it and
scan the new QR. Remove the old linked device from an official Signal client
only after the replacement works.

### Remove installed files

Fully quit Pidgin before removing either library. A Debian package can be
removed with the package manager. A CMake source install has no automated
uninstall target: preserve the manifest when installing each revision and use
the manifest from the currently installed build. Verify its prefix, then remove
exactly the files it lists. Do not remove a plugin directory recursively, and
do not use a manifest from another prefix or revision. Remove both
`libsignal-purple.so` and its private
`signal-purple/libsignal_core.so` from the same installation scope.

Removing installed files leaves per-user account data intact.

### Remove account data and the linked device

Complete account removal is separate and irreversible. First disable and remove
the Purple account. Delete its database under `~/.purple/signal-purple/` or the
configured custom path, delete the matching labelled item from the desktop
secret service, and remove the linked device from an official Signal client.
Never delete only the database or only its secret if recovery may still be
needed.
