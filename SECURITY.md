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

There is no supported release yet. Security fixes target the latest `main`
revision until the first tagged release. The project may need urgent dependency
updates when Signal changes its service or cryptographic stack.

## Scope

The project protects the local Presage database with SQLCipher and stores the
passphrase through libsecret. It does not yet expose a complete safety-number
verification workflow and has not undergone an independent audit. Read the
[security model](docs/security-model.md) for explicit guarantees and gaps.
