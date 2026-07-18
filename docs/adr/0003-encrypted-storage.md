# ADR 0003: SQLCipher plus libsecret

Status: accepted, 2026-07-18.

Presage state is stored in its SQLCipher-enabled SQLite backend. Each Purple
account gets a random 256-bit passphrase stored through libsecret. Failure to
read or write the secret fails the connection; there is no plaintext fallback.

Purple account preferences contain the device name, optional store path, and a
random non-secret store identifier used to keep the local database/keyring
lookup stable when an account label changes. They never contain linked-device
credentials or database keys.
