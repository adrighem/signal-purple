# ADR 0001: Own a polling C ABI

Status: accepted 2026-07-18; notifier amended 2026-07-20.

The Purple adapter must not bind upstream libsignal bridge symbols. The project
owns a small versioned ABI between C and Rust. Rust signals a private
nonblocking descriptor after queueing an event; the GLib main loop then polls
fully owned events and frees them explicitly. Rust never invokes a C callback
from its worker thread.

This avoids upstream ABI drift and callback teardown races. The original 20 ms
timer caused about 50 needless main-loop wakeups per second per account, so ABI
v6 replaced it with descriptor-driven readiness while retaining the owned,
nonblocking poll function and explicit normalization code.
