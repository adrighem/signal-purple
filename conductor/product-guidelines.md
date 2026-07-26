# Product Guidelines

## Voice and Tone

- Be concise, factual, and explicit that the plugin is unofficial and alpha
  quality.
- Present failures as actionable user outcomes without exposing sensitive
  identifiers or protocol details.

## Visual Identity

- Follow the host Purple UI. Do not introduce a separate visual system.

## Content Rules

- Use "Signal", "Pidgin", "Purple", and "linked device" consistently.
- Never log credentials, provisioning URIs, message bodies, account
  identifiers, or complete environment data.
- Keep public errors short and put diagnostic detail only in safe debug output.

## UX Principles

- A successful UI action must match the backend outcome.
- Cancellation, disconnect, and retry behavior must be predictable.
- Never claim a message or file was handled if the corresponding durable or
  remote action was not accepted.
- Preserve local aliases and other user-owned Purple state.
