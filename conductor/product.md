# Initial Concept

Provide a safe, maintainable Signal linked-device protocol plugin for Pidgin
and libpurple 2, with Purple-facing code in C and protocol/storage work behind
an owned Rust ABI.

# Product Overview

## Target Users

- Debian 13 users who need Signal messaging inside Pidgin.
- Maintainers who need explicit ownership, recovery, and compatibility
  boundaries around an unsupported third-party Signal stack.

## Goals

- Deliver synchronized direct and group messaging without exposing Signal
  cryptography or unstable Presage objects to C.
- Fail visibly and recoverably under network, memory, queue, and lifecycle
  pressure.
- Keep credentials, message state, and group secrets within the documented
  security boundaries.

## Key Features

- Linked-device provisioning and encrypted SQLCipher storage.
- Contact and group synchronization, durable text outbox, message replay, and
  bounded attachment handling.
- Versioned owned C/Rust ABI with descriptor-driven event delivery.

## Non-Goals

- Primary registration, calls, backups, contact discovery, or complete parity
  with official Signal clients.
- Support for Purple 3 or platforms outside the documented baseline.

## Success Metrics

- The canonical Rust and C checks pass on the supported toolchain.
- No accepted control action is silently lost under bounded work-queue pressure.
- Connection teardown does not retain unsafe frontend pointers or plaintext
  database credentials.
- Documented attachment limits apply before unbounded local allocation.
