# Debian packaging notes

Configure with `-DCMAKE_INSTALL_PREFIX=/usr` for a system package. The CMake
install layout then places:

- `libsignal-purple.so` in the GNUInstallDirs library path under `purple-2`;
- `libsignal_core.so` in its private `purple-2/signal-purple` subdirectory;
- protocol icons in `share/pixmaps/pidgin/protocols/{16,22,48}`;
- AppStream metadata in `share/metainfo`.

Override `SIGNAL_PURPLE_PLUGIN_DIR` when a multiarch package needs the exact
directory reported by:

```sh
pkg-config --variable=plugindir purple
```

The release source artifact contains the complete locked Cargo graph. Generate
it from a clean release commit with:

```sh
ionice -c 3 nice scripts/make-source-archive.sh HEAD
```

The script uses `cargo vendor`, writes Cargo source replacement into the
archive, and replaces the `debian/copyright` vendor placeholder with an exact
per-package DEP-5 inventory derived from each Cargo manifest. Archive metadata
is normalized to the commit timestamp before emitting a deterministic
`.orig.tar.xz`. Cargo is forced offline by the Debian rules, and CMake adds
`--offline` whenever the source tree contains `vendor/`.

Build the package by extracting the archive and using `debian/` from the same
commit. An audited Debian backport of Rust 1.94 or newer satisfies the declared
build dependencies and permits the normal command:

```sh
ionice -c 3 nice dpkg-buildpackage --build=binary --no-sign
```

Release evidence must come from a clean Debian 13 amd64 environment. A build on
a newer Debian host is useful only as a development check. The current protocol
graph requires Rust 1.94 while Debian 13 supplies Rust 1.85, so the package
currently requires an audited Debian backport or the checksum-verified upstream
Rust 1.95 toolchain. The clean-environment validation uses the upstream
`rust-1.95.0-x86_64-unknown-linux-gnu.tar.gz` distribution with SHA-256
`a47ac940abd12399d59ad15c877e7113fa35f2b9ec7e6a8a045d4fd8b9741dea`.

An upstream toolchain archive does not register as a Debian package even when
`rustc --version` and `cargo --version` report the required compiler. The
package metadata therefore defines the
`pkg.signal-purple.upstream-rust` build profile, which disables only the
Debian-package requirements for `cargo` and `rustc`. When an audited release
build uses the upstream archive, verify its checksum, install every non-Rust
build dependency, record the toolchain versions, and activate that profile:

```sh
echo 'a47ac940abd12399d59ad15c877e7113fa35f2b9ec7e6a8a045d4fd8b9741dea  rust-1.95.0-x86_64-unknown-linux-gnu.tar.gz' \
  | sha256sum --check
rustc --version --verbose
cargo --version --verbose
ionice -c 3 nice dpkg-buildpackage --build=binary --no-sign \
  --build-profiles=pkg.signal-purple.upstream-rust
```

All native build dependencies remain enforced by `dpkg-checkbuilddeps`; the
release build must not use `--no-check-builddeps`. The normal Debian 13 CI job
builds, tests, and stages the CMake install on the supported userspace, but does
not replace the vendored, offline package and reproducibility evidence.

The release workflow runs the same process in the pinned Debian 13 image with
packages selected over HTTPS from the `20260722T000000Z` Debian snapshot. A
digest-pinned bootstrap image supplies the CA bundle needed before Debian's
`ca-certificates` package is installed:

```sh
scripts/build-release-artifacts.sh v0.2.2 dist/release
```

The helper creates the deterministic vendored source archive twice, builds
that archive twice with Cargo offline, compares the runtime and debug packages,
and retains only matching outputs. In Actions, source vendoring is allowed
network access, but both extracted package builds run in a network-disabled
container, followed by the install/probe check in a second fresh container.
The workflow generates a
normalized SPDX 2.3 SBOM from the extracted vendored source, writes and verifies
`SHA256SUMS`, creates GitHub build-provenance attestations, and uploads the
exact allowlisted files to a draft release. The SBOM catalogs the root
`Cargo.lock` graph and excludes duplicate discovery inside `vendor/`; the
generated DEP-5 inventory remains the licensing record for those vendored
sources. Syft itself is downloaded as a versioned archive and verified against
its pinned SHA-256 before execution.

Runtime dependencies include libpurple 2, GLib, GdkPixbuf, libsecret, OpenSSL,
and the native libraries linked by the bundled SQLCipher backend. Use
`dpkg-shlibdeps` on both installed shared libraries rather than maintaining that
list manually.

The reproducibility evidence and artifact hashes for the current audited run
are recorded in [live-validation.md](live-validation.md).
