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
