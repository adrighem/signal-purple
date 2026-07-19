# Troubleshooting

## Signal does not appear in Pidgin

Confirm both shared libraries are in the same Purple plugin directory:

```sh
pkg-config --variable=plugindir purple
ldd /path/to/libsignal-purple.so
```

Run Pidgin with `--debug` and look for loader errors. Build against the same
libpurple family used at runtime.

## The database key cannot be loaded

The plugin requires a Secret Service provider. Unlock the desktop keyring and
confirm libsecret applications can store a secret. It intentionally refuses to
open a plaintext store when the service is unavailable.

## Linking fails or the QR expires

Disable and re-enable a fresh account to request a new provisioning flow. Make
sure the phone has network access and uses a current official Signal release.
Never paste the provisioning URI into a public issue.

## The account stays “connecting”

The plugin waits for Presage's pending queue to empty before allowing sends.
Large initial syncs can take time. A backend error should appear in Purple; use
sanitized debug output when reporting it.

## The Signal buddy list is empty

Reconnect the Signal account and allow the requested contact synchronization
to finish. The plugin logs an identifier-free summary such as `Applied contact
snapshot: 46 contacts, 46 created, 0 removed` when run with `pidgin --debug`.
Do not include contact identifiers in a public report.

Signal does not expose contact presence. Current builds mark synchronized
contacts reachable while the account is connected so Pidgin's default offline
filter does not hide the entire `Signal` group. Older builds require **View >
Show Offline Buddies**.

## Messages sent while Pidgin was offline are missing

On reconnect, the plugin drains envelopes still queued by Signal for this
linked device. It cannot request arbitrary older conversation history or copy
history from the primary phone. Conversation logging is deliberately disabled,
so previously displayed messages are not reconstructed from Purple logs.

## Sending to a raw identifier fails

Direct chats use canonical Signal service identifiers synchronized into the
buddy list. Phone-number discovery is not implemented. Start with a synced
contact rather than entering a phone number.

## Identity changes block messages

The current backend rejects changed identity keys and does not expose a
per-contact approval workflow. Verification in an official client does not
change signal-purple's separate store. Communication with that contact remains
blocked in this release. An inbound message can be skipped by Presage before
the adapter receives an error event, while an outbound send reports only a
generic operation failure.

After independently verifying the change, the only current recovery is to use a
new configured store path, link it as a new device, and remove the old linked
device from the primary phone. Retain the old encrypted store until the new
device works; do not overwrite or delete it as a routine workaround.
