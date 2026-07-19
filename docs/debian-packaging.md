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

Build the package by extracting the archive, copying `debian/` from the same
commit if needed, and running:

```sh
ionice -c 3 nice dpkg-buildpackage --build=binary --no-sign
```

Release evidence must come from a clean Debian 13 amd64 environment. A build on
a newer Debian host is useful only as a development check. The current protocol
graph requires Rust 1.94 while Debian 13 supplies Rust 1.85, so the package
currently requires an audited backport of the pinned Rust toolchain. This is a
release blocker until the backport is reproducible or the graph supports the
stock toolchain.

Runtime dependencies include libpurple 2, GLib, libsecret, OpenSSL, and the
native libraries linked by the bundled SQLCipher backend. Use `dpkg-shlibdeps`
on both installed shared libraries rather than maintaining that list manually.
