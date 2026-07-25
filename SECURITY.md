# Security policy

## Reporting a vulnerability

Do not open a public issue for suspected vulnerabilities. Use GitHub's private
vulnerability reporting for `adrighem/signal-purple`. If that is unavailable,
contact the maintainer through the private address in the GitHub profile and
include `signal-purple security` in the subject.

Please include affected revisions, impact, reproduction steps, and whether any
credentials or real message content were involved. Do not send live Signal
keys, provisioning URIs, database passphrases, or private message contents.

## Supported versions

There is no supported stable release yet; all published versions are alpha
prereleases. Security fixes target the latest `main` revision. The maintainer
may publish an updated alpha release when a fix is suitable for users, but
older alpha versions do not receive backports. The project may need urgent
dependency updates when Signal changes its service or cryptographic stack.

## Scope

The project protects the local Presage database with SQLCipher and stores the
passphrase through libsecret. Identity replacements have a warning and
acceptance workflow, but numeric safety-number comparison still belongs in an
official Signal client. The project has not undergone an independent audit.
Read the [security model](docs/security-model.md) for explicit guarantees and
gaps.
