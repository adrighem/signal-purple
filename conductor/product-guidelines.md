# Product Guidelines

## Voice and Tone

- Be concise and factual. Describe the plugin as stable only within its
  documented supported scope, independent from Signal, and exposed to Signal
  service compatibility changes.
- Present failures as actionable user outcomes without exposing sensitive
  identifiers or protocol details.

## Visual Identity

- Follow the host Purple UI. Do not introduce a separate visual system.

## Content Rules

- Use "Signal", "Pidgin", "Purple", and "linked device" consistently.
- Never put credentials, provisioning URIs, message bodies, account
  identifiers, or complete environment data in diagnostic or error logs.
- Treat user-controlled Purple conversation transcripts as user-facing local
  storage, not diagnostic output. Preserve Purple's standard global and
  per-conversation controls without appending diagnostic-only protocol
  metadata.
- Keep public errors short and put diagnostic detail only in safe debug output.

## UX Principles

- A successful UI action must match the backend outcome.
- Cancellation, disconnect, and retry behavior must be predictable.
- Never claim a message or file was handled if the corresponding durable or
  remote action was not accepted.
- Preserve local aliases and other user-owned Purple state.
