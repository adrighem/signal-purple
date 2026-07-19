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

Current builds also replay messages which Presage saved but Purple did not
acknowledge before a crash. The acknowledgment is encrypted in the Signal
store and is written only after Purple accepts the message for display. This is
crash recovery, not a general history request.

## Sending to a raw identifier fails

Direct chats use canonical Signal service identifiers synchronized into the
buddy list. Phone-number discovery is not implemented. Start with a synced
contact rather than entering a phone number.

## A sent message reports a failure

Direct and group messages are stored in the encrypted outbox before the first
network attempt. A transient failure leaves the original message queued with
bounded exponential backoff, including across Pidgin restarts. Repeated
notifications at later attempt thresholds mean the message is still queued,
not that a new copy was created. Identity-blocked messages retry immediately
after the changed identity is accepted.

## Identity changes block messages

Incoming messages continue across an identity replacement to avoid losing an
envelope which Signal has already acknowledged. Sending also continues for a
contact that was not explicitly verified, with a one-time advisory.

For an explicitly verified contact, sending remains blocked. Verify the
contact through another trusted channel, then right-click that Signal buddy and
choose **Accept changed Signal identity**. The plugin resets the affected
sessions and marks the contact unverified, without relinking the account. Do
not accept merely to clear the warning.
