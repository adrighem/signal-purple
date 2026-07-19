# Dependency policy

Signal-stack dependencies are operationally sensitive and must use exact Git
revisions. Ordinary builds use `--locked`; dependency updates must include the
lockfile.

Before merging a Presage, libsignal-service-rs, libsignal, SQLCipher, or Tokio
update:

1. Review the complete upstream diff and provenance.
2. Inspect new build scripts, native code, network behavior, and licenses.
3. Run Rust formatting, clippy, unit tests, CMake build, C tests, and plugin
   probe on Debian 13.
4. Exercise store open/migration and teardown under sanitizers where possible.
5. Run the live compatibility matrix in `compatibility.md` with dedicated test
   accounts.
6. Record compatibility-impacting decisions in the changelog and maintainer
   decision log.

Automated update pull requests may open for visibility, but compilation alone
does not authorize merging them.

The Presage dependency currently uses the public
[`adrighem/presage`](https://github.com/adrighem/presage) fork at
`c8ee98cf944897812d9375effb98657f537e8b09`. Its parent is the previously
audited upstream pin `63482efd0cbdc0780baf0650517c7d55f1cac05d`; the fork adds
only read-only Storage Service group synchronization plus its error plumbing
and focused test. Treat any rebase or additional fork commit as a full
Signal-stack update under the policy above.
