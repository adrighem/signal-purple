# Attachment resource policy

These limits describe local resource ownership, not Signal service guarantees.
Incoming transfer size follows Signal's network policy; the plugin adds no
lower per-file or per-message download cap.

| Direction and stage | Limit | Owner | Outcome |
| --- | ---: | --- | --- |
| Incoming file | Signal network policy | Signal service and Presage | Accept any non-empty decrypted file delivered by Signal |
| Outgoing file | 25 MiB per file | Rust core and C adapter | Reject the file |
| Admitted outgoing files | 2 files and 50 MiB total per account | Rust core | Retryable queue-full result |
| Queued binary events | 64 MiB aggregate, with one larger event admitted alone | Rust event queue | Producer backpressure |
| Unresolved receive prompts | No plugin byte cap | C adapter | Retain data until the user accepts or rejects the prompt |
| Inline direct/group JPEG/PNG | 8 MiB encoded, 8192 pixels per edge, and 16 megapixels | Rust classifier and C image decoder | Fall back to a file prompt |
| Inline direct/group GIF | 8 MiB encoded, 8192 pixels per edge, 16 megapixels, and 8 million cumulative canvas pixel-frames | Rust classifier, C GIF parser, and image decoder | Fall back to a file prompt |
| Signal GIF-style MP4 conversion | 8 MiB input/output, 480 pixels per edge, 15 fps maximum, 2 attempts per message, 1 process globally | Rust worker, `prlimit`, and FFmpeg | Preserve the original MP4 file prompt |

Incoming downloads use Presage's standard attachment API. Signal decides which
encrypted attachment sizes the service accepts. The plugin rejects empty data
and download or decryption failures, but does not enforce sender-declared or
decrypted byte limits of its own.

Signal GIF-style conversion additionally runs with a cleared environment,
pipe-only FFmpeg input and output, one codec thread, a 1 GiB address-space
limit, a 10-second soft and 12-second hard CPU limit, a 15-second wall limit,
and 64 file descriptors. Generated GIFs must pass the same structure, byte,
dimension, and frame-area checks as native GIFs before replacing the original
presentation. No media temporary file is created. Missing helpers, contention,
process failure, timeout, invalid output, or any exceeded budget leaves the
downloaded MP4 unchanged.

Outgoing admission covers commands waiting in the core, commands deferred
during recovery, and active upload tasks. A non-cloneable permit owns both the
payload budget and request identifier through terminal event delivery; dropping
it on rejection, cancellation, panic, recovery failure, or shutdown restores
capacity automatically. Cancellation changes state directly on this permit,
without depending on bounded command-queue capacity, and the upload task
observes the same state before and during network work. The two-file admission
ceiling also bounds concurrent uploads.

The outgoing retained-payload limit is not a complete resident-memory bound.
The C adapter holds one bounded local-file copy while it crosses the FFI, and
upstream libraries may temporarily hold additional copies. The adapter opens
the path once, rejects non-regular and known-oversized inputs before allocating
their contents, and still stops after the limit plus one byte if the file grows.
Incoming attachments and unresolved receive prompts remain in memory without a
plugin aggregate byte cap, and FFI/UI handoff can temporarily hold additional
copies. Attachments are not stored in the durable text-message outbox and must
be sent again after a failed upload or restart.
