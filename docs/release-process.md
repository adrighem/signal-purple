# Release process

1. Confirm the roadmap scope and identify the applicable release-checklist
   gates.
2. Land every intended runtime, storage, dependency, packaging, CI, and release
   hardening change before freezing the candidate.
3. Open one candidate-validation issue to record the environment, evidence,
   and blockers. Leave its candidate revision pending at this stage.
4. Let release-please create or update the version and changelog pull request.
5. Review every dependency and generated-file change, then merge the release
   pull request only when its checks pass. This merge establishes the candidate
   on `main`; it does not create a tag or GitHub release.
6. Record the full post-merge `main` commit in the validation issue. Evidence
   from the pull-request head does not substitute for this commit.
7. Pass the clean Debian 13 job, sanitizer checks, and live compatibility
   matrix for that candidate using non-production accounts.
8. Vendor the locked Rust sources and prove two reproducible offline package
   builds from that candidate.
9. Verify install, load, upgrade, rollback, and uninstall paths, then complete
   the agreed soak period without a release blocker.
10. Create and push a verified signed tag for the validated commit.
    Release-please intentionally does not create tags or GitHub releases in
    this repository.
11. Send the default-branch-bound `prepare-release` repository dispatch for
    the signed tag:

    ```sh
    gh api --method POST repos/adrighem/signal-purple/dispatches \
      -f event_type=prepare-release \
      -F 'client_payload[tag]=v0.2.2'
    ```

    The workflow verifies the tag and all version files, reproduces the source
    archive and Debian packages twice, installs and probes the package,
    generates the SBOM and checksums, attests the artifacts, and attaches them
    to a draft GitHub release.
12. Download and verify the draft assets, complete any required manual
    checksum signature, and publish the release with explicit known
    limitations and the exact tested Signal client dates.

The workflow uses `RELEASE_PLEASE_TOKEN` when configured and otherwise falls
back to `GITHUB_TOKEN`. Configure a fine-grained token or GitHub App token when
release pull requests must trigger other workflows automatically; events made
with the repository `GITHUB_TOKEN` do not start new workflow runs.

The release-artifacts workflow runs from the trusted default-branch definition
when a release is published or when the `prepare-release` repository dispatch
is sent to prepare a draft first. It deliberately does not use
`workflow_dispatch`, which can execute a modified workflow from another ref.
It accepts only a canonical `vMAJOR.MINOR.PATCH` annotated tag after verifying
it with the sole public key in
`keys/release-signing-key.asc`, the signature status, and the pinned fingerprint
`B3C0B75FA3B33AC278738C5CB1798BCDA76054BD`. The tag must identify a commit on
`main`, and the tag, release manifest, Cargo files, citation metadata, and
`version.txt` must agree.

Build, attestation, and publication use separate jobs. Only the final job can
write repository contents, and it does not check out or execute repository
code. Existing assets are skipped only when their GitHub-reported SHA-256
digest agrees; a conflicting asset fails the run and is never overwritten.
Payloads are uploaded before `SHA256SUMS`, making an interrupted run safely
resumable by rerunning the same workflow. Another `prepare-release` dispatch
resumes a draft; a published-release run must be rerun from its original
event.

An explicit run creates or resumes a draft, which is the preferred path because
it keeps a failed or incomplete artifact build from becoming public. The
`release: published` trigger is a recovery guard: it populates a release when
the draft step was skipped, although its assets will appear only after the
workflow completes. The maintainer's private OpenPGP key is never stored in
Actions. The signed tag authenticates the source, and GitHub's OIDC-backed
artifact attestation authenticates the automated build. If a detached
`SHA256SUMS.asc` is required, create and upload it locally before publishing
the draft.

Do not publish a release from a working tree with only compilation evidence.

## Candidate validation tracker

Use one issue as the evidence index for each release candidate. The 0.2.0
pre-release candidate is tracked in
[issue #5](https://github.com/adrighem/signal-purple/issues/5).
Record the full post-merge `main` commit, Debian image or environment, official
Signal client versions and test date, artifact hashes, and links to sanitized
evidence. Keep the issue open through packaging, signing, and publication; a
release pull request must not close it automatically.

Evidence counts only for the recorded candidate. If runtime, storage,
dependency, or packaging inputs change, update the candidate revision and rerun
the affected checks. A pull-request head and its resulting merge commit may have
the same tree but remain different revisions and can produce different artifact
metadata. Use dedicated non-production Signal accounts, keep failed checks
open, and link implementation defects instead of creating separate validation
trackers. Never attach identifiers, message contents, provisioning data, keys,
database secrets, or unredacted private paths.

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
