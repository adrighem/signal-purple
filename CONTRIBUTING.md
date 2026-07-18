# Contributing

Contributions are welcome, especially focused tests, lifecycle fixes,
documentation improvements, and compatibility updates.

1. Open an issue before large protocol or storage changes.
2. Keep all Purple calls on the GLib main thread.
3. Keep upstream Signal APIs behind the owned C ABI.
4. Add or update tests for behavior changes.
5. Run the checks in [docs/development.md](docs/development.md).
6. Use SPDX headers and preserve all third-party notices.

Do not use real account identifiers, message text, provisioning URIs, or key
material in tests, fixtures, logs, issues, or pull requests.

By contributing, you agree that C/project contributions are provided under
GPL-3.0-or-later and Rust backend contributions under AGPL-3.0-only, matching
the file-level SPDX identifier.
