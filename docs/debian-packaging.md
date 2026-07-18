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

The current Cargo graph includes Git sources and therefore is not yet suitable
for a network-isolated Debian build. A release source artifact must vendor the
locked Rust graph and configure Cargo source replacement. Do not claim official
Debian packaging until that artifact builds in a clean Debian 13 environment.

Runtime dependencies include libpurple 2, GLib, libsecret, OpenSSL, and the
native libraries linked by the bundled SQLCipher backend. Use `dpkg-shlibdeps`
on both installed shared libraries rather than maintaining that list manually.
