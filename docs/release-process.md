# Release process

1. Confirm the roadmap scope and complete the applicable release checklist.
2. Use one candidate-validation issue to record the exact revision, test
   environment, evidence, and blockers.
3. Let release-please create or update the version and changelog pull request.
4. Review every dependency and generated-file change, then merge the release
   pull request only when its checks pass.
5. Pass the live compatibility matrix using non-production accounts.
6. Vendor the locked Rust sources and prove a network-isolated Debian 13 build.
7. Build the source archive and package from the reviewed release commit.
8. Create and push a verified signed tag for that exact commit. Release-please
   intentionally does not create tags or GitHub releases in this repository.
9. Generate checksums and an SBOM, then verify install, load, upgrade, and
   uninstall paths.
10. Publish the GitHub release with explicit known limitations and the exact
   tested Signal client dates.

The workflow uses `RELEASE_PLEASE_TOKEN` when configured and otherwise falls
back to `GITHUB_TOKEN`. Configure a fine-grained token or GitHub App token when
release pull requests must trigger other workflows automatically; events made
with the repository `GITHUB_TOKEN` do not start new workflow runs.

Do not publish a release from a working tree with only compilation evidence.

## Candidate validation tracker

Use one issue as the evidence index for each release candidate. The 0.2.0
pre-release candidate is tracked in
[issue #5](https://github.com/adrighem/signal-purple/issues/5).
Record the full candidate commit, Debian image or environment, official Signal
client versions and test date, artifact hashes, and links to sanitized evidence.

Evidence counts only for the recorded candidate. If runtime, storage,
dependency, or packaging inputs change, update the candidate revision and rerun
the affected checks. Use dedicated non-production Signal accounts, keep failed
checks open, and link implementation defects instead of creating separate
validation trackers. Never attach identifiers, message contents, provisioning
data, keys, database secrets, or unredacted private paths.

## Upgrade

1. Disable the Signal account and close Pidgin so the encrypted database is
   quiescent.
2. Keep a copy of the database path shown in the account's advanced settings.
   The matching secret-service entry is labelled `signal-purple database for
   <account>` and is required to open that copy.
3. Install the complete new package or both libraries from one source build.
   Never mix a plugin from one revision with a backend library from another.
4. Start Pidgin, enable the account, and confirm it reconnects without a QR,
   then confirm contacts, groups, and a direct send/receive round trip.

Store migrations are automatic and additive. Keep the pre-upgrade database and
secret until the new version has completed its validation period.

## Rollback

Close Pidgin and reinstall the previous complete package or both libraries from
the previous source build. Restore the matching pre-upgrade database only if the
older version cannot open the upgraded copy. Do not replace one of the two
shared libraries independently. If a rollback cannot reconnect, return to the
new build and its database rather than deleting state. Relinking is the last
recovery option because it creates a new Signal linked device.

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
uninstall target: keep and inspect that build directory's
`install_manifest.txt`, then remove exactly the files it lists. Do not remove a
plugin directory recursively, and do not use a manifest from another prefix or
revision. Remove both `libsignal-purple.so` and its private
`signal-purple/libsignal_core.so` from the same installation scope.

Removing installed files leaves per-user account data intact.

### Remove account data and the linked device

Complete account removal is separate and irreversible. First disable and remove
the Purple account. Delete its database under `~/.purple/signal-purple/` or the
configured custom path, delete the matching labelled item from the desktop
secret service, and remove the linked device from an official Signal client.
Never delete only the database or only its secret if recovery may still be
needed.
