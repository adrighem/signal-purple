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
