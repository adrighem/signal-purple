# ADR 0001: Own a polling C ABI

Status: accepted, 2026-07-18.

The Purple adapter must not bind upstream libsignal bridge symbols. The project
owns a small versioned ABI between C and Rust. C polls fully owned events and
frees them explicitly; Rust never invokes a C callback from its worker thread.

This avoids upstream ABI drift and removes callback teardown races. The cost is
a short polling interval and explicit normalization code, which is acceptable
for a desktop messaging plugin.
