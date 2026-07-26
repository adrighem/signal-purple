# General Code Style Principles

## Readability

- Prefer explicit ownership and state transitions over clever control flow.
- Make invalid lifecycle and resource states difficult to represent.

## Consistency

- Follow existing naming, formatting, and error-reporting patterns.
- Preserve SPDX identifiers and file-level licensing.

## Simplicity

- Keep reliability-critical control paths independent from best-effort work.
- Refactor only when it directly clarifies an invariant under test.

## Maintainability

- Add regression coverage at the boundary where a failure was observed.
- Keep queue capacity, memory ownership, and cancellation semantics explicit.

## Documentation

- Document why an invariant exists and keep architecture/security documents in
  sync with behavioral changes.
