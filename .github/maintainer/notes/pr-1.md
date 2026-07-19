# PR #1: bump actions/checkout to v7.0.0

- **Intent:** update both workflows from `actions/checkout` v4.3.1 to v7.0.0.
- **Provenance:** Dependabot pins the official `v7.0.0` tag commit,
  `9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0`.
- **Scope:** two one-line workflow changes; `persist-credentials: false` remains
  unchanged.
- **Compatibility:** v7 uses Node.js 24 and requires Actions Runner v2.327.1 or
  later. This repository uses GitHub-hosted `ubuntu-latest` runners.
- **Validation:** CI `build-and-test` and dependency review both pass on the PR.
- **Outcome:** merged as commit `5fa96c6` after human approval.
- **Unknowns:** self-hosted runner compatibility is irrelevant unless the
  workflows move away from GitHub-hosted runners.
