# Release process

1. Confirm the roadmap scope and update the changelog.
2. Review every dependency change and run all local checks.
3. Pass the live compatibility matrix using non-production accounts.
4. Vendor the locked Rust sources and prove a network-isolated Debian 13 build.
5. Build the source archive and package from a clean signed tag.
6. Generate checksums and an SBOM, then verify install, load, upgrade, and
   uninstall paths.
7. Publish explicit known limitations and the exact tested Signal client dates.

Do not publish a release from a working tree with only compilation evidence.
