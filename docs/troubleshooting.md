# Troubleshooting

## Signal does not appear in Pidgin

For a system install, confirm the Purple plugin and its private backend are in
the expected relative locations:

```sh
plugin_dir="$(pkg-config --variable=plugindir purple)"
plugin="$plugin_dir/libsignal-purple.so"
backend="$plugin_dir/signal-purple/libsignal_core.so"

test -r "$plugin"
test -r "$backend"
ldd "$plugin" | grep -E 'libsignal_core|not found'
```

`ldd` must resolve `libsignal_core.so` from the plugin's private
`signal-purple` subdirectory and must not report it as missing. A per-user copy
can shadow the system plugin, so check both scopes when the result is unexpected:

```sh
profile="${PURPLEHOME:-$HOME/.purple}"
find "$profile/plugins" "$plugin_dir" -type f \
  \( -name 'libsignal-purple.so' -o -name 'libsignal_core.so' \) -print \
  2>/dev/null
```

Use only one installation scope at a time and install both libraries from the
same build. Fully quit every Pidgin process after replacing either library, then
run Pidgin with `--debug` and look for loader errors. Build against the same
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

## Pidgin uses excessive CPU while Signal is idle

First confirm the hot process and thread rather than assuming every Pidgin CPU
spike comes from Signal. Current builds use descriptor-driven event delivery and
do not poll the backend on a fixed timer. After upgrading both plugin libraries,
fully restart Pidgin so it does not keep an older deleted library mapping, then
sample again. If the main Pidgin thread or `signal-purple-core` remains hot, run
`pidgin --debug` and report which thread is busy; sanitize all identifiers and
provisioning data before sharing output.

## The Signal buddy list is empty

Reconnect the Signal account and allow the requested contact synchronization
to finish. The plugin logs an identifier-free summary such as `Applied contact
snapshot: 46 contacts, 46 created, 0 removed` when run with `pidgin --debug`.
Do not include contact identifiers in a public report.

Signal does not expose contact presence. Current builds mark synchronized
contacts reachable while the account is connected so Pidgin's default offline
filter does not hide them in the default buddy group. Older builds require
**View > Show Offline Buddies**.

## Contacts or chats remain in the old Signal groups

Reconnect the account and let contact and group synchronization finish. The
plugin adopts an unmarked contact in the exact legacy `Signal` group only when
the current Signal snapshot confirms the same account and identifier. It then
moves adopted contacts and managed chats from `Signal` and `Signal groups` into
Purple's localized default buddy and chat groups. It does not move custom
placements or unmatched user-created nodes. After moving or removing its last
managed node, the plugin removes the legacy group if it is empty.

## Leaving or removing a Signal group

To change Signal membership, right-click an active managed chat, choose **Leave
Signal group…**, and confirm. The chat remains in Pidgin if the remote request
fails; it is closed and removed only after the backend reports success.

Pidgin's built-in **Remove Chat** operation is local-only because Purple 2 does
not give protocol plugins a removal callback. A removed managed chat can
therefore return after the next group synchronization. Closing the conversation
tab is also local-only. Neither action leaves the Signal group.

If **Leave Signal group…** is absent, allow a complete group refresh to finish.
The plugin offers remote leave only for a synchronized group in which the
account is currently an active member. Run `pidgin --debug` to inspect a failure,
which reports identifier-free queued/completed/failed leave milestones. Sanitize
any contact, group, or request identifiers from surrounding output before
sharing the log.

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

## An attachment is rejected or fails

Each attachment is limited to 25 MiB, and one incoming message may contain at
most 50 MiB. Save or reject existing receive prompts before retrying if Purple
reports that 64 MiB is already waiting for a destination. Remote filenames are
sanitized and decrypted bytes are held in memory rather than a plugin-managed
temporary cache. Outgoing file uploads can be cancelled, but they are not kept
in the persistent text-message outbox, so send the file again after a restart or
failed upload.

Incoming group JPEG and PNG images are shown inline when their MIME type and
file signature agree, the complete image decodes, and it is no larger than 8192
pixels per edge or 16 megapixels total. Direct images, other image formats,
ordinary files, invalid or oversized images, and content with mismatched
metadata still use a receive prompt. If an eligible group JPEG or PNG still
opens as a direct transfer after upgrading, fully quit and restart Pidgin so the
old plugin module is unloaded before testing again.
