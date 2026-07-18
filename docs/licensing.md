# Licensing

The project uses a documented file/component boundary:

- C plugin, public header, general documentation, and project icon:
  **GPL-3.0-or-later**.
- Rust `signal-core` backend: **AGPL-3.0-only**, matching the Presage,
  libsignal-service-rs, and libsignal code it links.
- AppStream metadata and tool configuration may use CC0-1.0 where their SPDX
  header says so.

GNU GPLv3 section 13 permits GPLv3-covered code to be combined with AGPLv3
code. The GPL continues to apply to the GPL-covered portion, while AGPL section
13 requirements apply to the combination as a whole. Consequently, it would be
misleading to distribute the two combined libraries as “GPL only.”

This structure honors the preference for GPLv3 on original Purple-side code
without concealing the stronger obligations of the backend. See
[THIRD_PARTY_LICENSES.md](../THIRD_PARTY_LICENSES.md), the root GPL text, and
the AGPL text under `LICENSES/`.

This document is an engineering record and not legal advice.
