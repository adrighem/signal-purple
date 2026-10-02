# Dependency policy

Signal-stack dependencies are operationally sensitive. Presage and
libsignal-service-rs track their fork's `main` branch (no `rev` pin); ordinary
builds use `--locked` against the committed lockfile, so day-to-day builds
stay reproducible and only an explicit `cargo update` moves the pin forward.
Any such update must include the lockfile.

Before merging a Presage, libsignal-service-rs, libsignal, SQLCipher, or Tokio
update:

1. Review the complete upstream diff and provenance.
2. Inspect new build scripts, native code, network behavior, and licenses.
3. Run Rust formatting, clippy, unit tests, CMake build, C tests, and plugin
   probes on Debian 13 and Ubuntu 24.04 LTS.
4. Exercise store open/migration and teardown under sanitizers where possible.
5. Run the live compatibility matrix in `compatibility.md` with dedicated test
   accounts.
6. Record compatibility-impacting decisions in the changelog and maintainer
   decision log.

Automated update pull requests may open for visibility, but compilation alone
does not authorize merging them.

The Presage dependency currently uses the public
[`adrighem/presage`](https://github.com/adrighem/presage) fork's `main` branch.
The fork carries the Storage Service group refresh needed to build and
atomically reconcile an authoritative active set, the remote group-leave
operation, and a single-connection SQLx pool which serializes writes within
each store pool. Its file-backed regression holds one write transaction,
proves a second write queues rather than returning `SQLITE_BUSY`, then
verifies completion after the first transaction commits. Main also bounds
encrypted attachment reads and preserves profile-derived contact data. These
were previously carried only on a pinned rev off an old fork point; they are
now ordinary commits on `main`, merged alongside upstream `whisperfish/presage`
history (the libsignal 0.99.0 / sqlx 0.9 upgrade), so the fork no longer
diverges from upstream except by these commits. Its nested libsignal-service
dependency is the public
[`adrighem/libsignal-service-rs`](https://github.com/adrighem/libsignal-service-rs)
fork's `main` branch, whose only fork-only change preserves Storage response
keys for exact completeness validation, likewise merged onto `main` rather
than pinned. Upstream `presage-store-sqlite` (unforked) does not carry the
single-connection pool serialization fix; that is fork-only. Treat any
additional fork commit, or a `cargo update` that advances either pin, as a
full Signal-stack update under the policy above.
