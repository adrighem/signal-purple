# Rust Style

- Format with the pinned rustfmt configuration and compile without Clippy
  warnings.
- Keep unsafe operations inside the FFI module with explicit safety comments.
- Prefer RAII and typed state over parallel IDs, byte counts, and flags.
- Keep shutdown and cancellation observable at every potentially blocking
  await boundary.
- Use deterministic unit tests for queue pressure, cancellation, and timestamp
  allocation.
