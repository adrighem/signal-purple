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
[`adrighem/presage`](https://github.com/adrighem/presage) fork at the exact Git
revision `5e584595b3723e6904a09246deaa830b93bbae7b`. The fork carries the
Storage Service group refresh needed to build and atomically reconcile an
authoritative active set, together with the remote group-leave operation and
focused tests. Its nested libsignal-service dependency is the public
[`adrighem/libsignal-service-rs`](https://github.com/adrighem/libsignal-service-rs)
fork at `c41a2d0332634dd3cbc830d0d4a77bdc0e9d2cae`, whose only fork change
preserves Storage response keys for exact completeness validation. Treat any
rebase or additional fork commit as a full Signal-stack update under the policy
above.
