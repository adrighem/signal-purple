# Compatibility

## Supported baseline

- Debian 13 userspace
- libpurple 2.14.x
- Pidgin 2 or another Purple 2 UI with request-field support
- Rust 1.94+ for building (repository pin: 1.95.0)
- A working Secret Service implementation accessible through libsecret
- A current Signal account on Android or iOS for linked-device approval

Purple 3 is a different API and is not supported. Primary-device desktop
registration is not supported.

## Signal service drift

Signal's specifications are public, but its production service is not a stable
third-party API. The official libsignal bridges also do not promise external
compatibility. Even a previously working signal-purple build can stop linking,
receiving, or sending after a service change.

The project pins Presage and the lower stack to make each revision
reproducible. It does not silently float dependencies. Scheduled or release
validation should test, with non-production accounts:

1. fresh QR linking;
2. reconnect with an existing encrypted store;
3. direct send/receive in both directions;
4. group send/receive;
5. typing and delivery receipts;
6. contact buddy-list creation, alias updates, and stale-contact removal;
7. primary-phone and second linked-device synchronization.

No revision should be called compatible until those checks pass.
Completed and outstanding scenarios are recorded in
[live-validation.md](live-validation.md).
