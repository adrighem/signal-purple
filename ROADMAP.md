# Roadmap

The project advances only when the acceptance criteria for a milestone are
demonstrated. Version numbers are compatibility targets, not deadlines.

## Milestone 0: release contract

- Define the supported 1.0 scope and objective release gates.
- Record evidence for every release decision.

## Milestone 1: v0.1.0 foundation

- Keep versions synchronized with release-please while signed tags remain a
  deliberate release action.
- Produce a reproducible Debian 13 package.
- Complete contact, group, offline-message, and resilience testing against real
  Signal clients.

## Milestone 2: v0.2.0 semantics and trust

- Surface identity changes without relinking or silent trust.
- Let the user accept a changed identity and resume from the same conversation.
- Complete read, delivery, typing, and retry semantics.

## Milestone 3: v0.3.0 attachments

- Add bounded attachment transfer with safe temporary storage.
- Verify cancellation, malformed input, and resource limits.

## Milestone 4: 1.0 release

- Pass every item in [the 1.0 checklist](docs/release-checklist.md).
- Publish a release candidate and complete a soak period.
- Publish 1.0 only from the reviewed release commit.
