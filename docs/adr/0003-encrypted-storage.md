# ADR 0003: SQLCipher plus libsecret

Status: accepted, 2026-07-18.

Presage state is stored in its SQLCipher-enabled SQLite backend. Each Purple
account gets a random 256-bit passphrase stored through libsecret. Failure to
read or write the secret fails the connection; there is no plaintext fallback.
The C adapter frees its libsecret allocation immediately after core
construction. The Rust FFI copy immediately enters zeroizing ownership and is
wiped as soon as the SQLCipher open attempt succeeds, fails, or is interrupted;
it is not retained for the linked-device session.

Purple account preferences contain the device name, optional store path, and a
random non-secret store identifier used to keep the local database/keyring
lookup stable when an account label changes. They never contain linked-device
credentials or database keys.
