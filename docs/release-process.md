# Release process

1. Confirm the roadmap scope and complete the applicable release checklist.
2. Let release-please create or update the version and changelog pull request.
3. Review every dependency and generated-file change, then merge the release
   pull request only when its checks pass.
4. Pass the live compatibility matrix using non-production accounts.
5. Vendor the locked Rust sources and prove a network-isolated Debian 13 build.
6. Build the source archive and package from the reviewed release commit.
7. Create and push a verified signed tag for that exact commit. Release-please
   intentionally does not create tags or GitHub releases in this repository.
8. Generate checksums and an SBOM, then verify install, load, upgrade, and
   uninstall paths.
9. Publish the GitHub release with explicit known limitations and the exact
   tested Signal client dates.

The workflow uses `RELEASE_PLEASE_TOKEN` when configured and otherwise falls
back to `GITHUB_TOKEN`. Configure a fine-grained token or GitHub App token when
release pull requests must trigger other workflows automatically; events made
with the repository `GITHUB_TOKEN` do not start new workflow runs.

Do not publish a release from a working tree with only compilation evidence.

## Upgrade

1. Disable the Signal account and close Pidgin so the encrypted database is
   quiescent.
2. Keep a copy of the database path shown in the account's advanced settings.
   The matching secret-service entry is labelled `signal-purple database for
   <account>` and is required to open that copy.
3. Install the complete new package. Never mix a plugin from one revision with
   a backend library from another revision.
4. Start Pidgin, enable the account, and confirm it reconnects without a QR,
   then confirm contacts, groups, and a direct send/receive round trip.

Store migrations are automatic and additive. Keep the pre-upgrade database and
secret until the new version has completed its validation period.

## Rollback

Close Pidgin and reinstall the previous complete package. Restore the matching
pre-upgrade database only if the older version cannot open the upgraded copy.
Do not replace one of the two shared libraries independently. If a rollback
cannot reconnect, return to the new package and its database rather than
deleting state. Relinking is the last recovery option because it creates a new
Signal linked device.

The release owner decides to roll back when an upgrade cannot load, reconnect,
or preserve the buddy/group projection, or when a security or message-delivery
regression is found. Release artifacts and the previous package must remain
available until the soak period ends.

## Relink and removal

To relink without destroying recoverable state, disable the account and choose
a new empty encrypted-store path in its advanced settings. Re-enable it and
scan the new QR. Remove the old linked device from an official Signal client
only after the replacement works.

Removing the Debian package leaves per-user data intact. For a complete and
irreversible removal, first disable/remove the account, uninstall the package,
delete its database under `~/.purple/signal-purple/` or the configured custom
path, and delete the matching labelled item from the desktop secret service.
Also remove the linked device from an official Signal client. Never delete only
the database or only its secret if recovery may still be needed.
