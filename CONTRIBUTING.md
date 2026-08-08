# Contributing

Contributions are welcome, especially focused tests, lifecycle fixes,
documentation improvements, and compatibility updates.

## Before you start

1. Open an issue before large protocol, storage, ABI, or user-data lifecycle
   changes.
2. Keep all Purple calls on the GLib main thread.
3. Keep upstream Signal APIs behind the owned C ABI.
4. Add or update tests for behavior changes.
5. Use SPDX headers and preserve all third-party notices.

The main ownership boundaries are:

| Area | Location | Responsibility |
| --- | --- | --- |
| Purple adapter | `src/` | GLib/Purple UI, lifecycle, contacts, groups, and transfers |
| Owned C ABI | `include/signal_core.h`, `rust/signal-core/src/ffi.rs` | Versioned cross-language validation and ownership |
| Signal backend | `rust/signal-core/src/backend.rs` | Presage actor, storage, synchronization, and projection |
| Event delivery | `rust/signal-core/src/event_queue.rs` | Bounded events and descriptor notification |
| Focused and integration tests | `tests/`, Rust module tests | C helpers, real plugin loading, ABI, and backend rules |

See [development.md](docs/development.md) for targeted commands and architecture
rules.

## Pull requests and releases

Use a concise Conventional Commit-style pull-request title, such as
`fix: handle an interrupted attachment`, `feat: add group action`, or
`docs: clarify installation`. Use `!` and describe `BREAKING CHANGE:` only for
an intentional compatibility break. Accurate `fix:` and `feat:` titles become
release notes; maintenance-only types should not pretend to be user-facing
changes.

Release Please owns `CHANGELOG.md`, `version.txt`,
`.release-please-manifest.json`, the Rust package version and lockfile version,
and `CITATION.cff`. Do not edit these in an ordinary pull request unless the
change explicitly repairs release metadata. The generated release pull request
updates them together.

Maintainers should use the [release process](docs/release-process.md) for
candidate validation, automation, and publication. User installation lifecycle
instructions belong in the [README](README.md#installation-lifecycle), not in
the maintainer procedure.

## Validation

Run the fast feedback loop while iterating:

```sh
scripts/check.sh fast
```

If its C formatting phase fails, apply the repository's pinned style with
`scripts/check.sh c-format-fix`.

Before requesting review, run the complete local gate:

```sh
scripts/check.sh full
```

The Debian 13 and Ubuntu 24.04 LTS CI jobs are the authoritative clean-platform
build and install probes. Document any check that cannot run locally and why.

Do not use real account identifiers, message text, provisioning URIs, or key
material in tests, fixtures, logs, issues, or pull requests.

By contributing, you agree that C/project contributions are provided under
GPL-3.0-or-later and Rust backend contributions under AGPL-3.0-only, matching
the file-level SPDX identifier.
