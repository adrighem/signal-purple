# Attachment resource policy

These are local defensive limits, not Signal service guarantees. They bound
different ownership stages independently and may change without a C ABI bump.

| Direction and stage | Limit | Owner | Outcome |
| --- | ---: | --- | --- |
| Incoming or outgoing file | 25 MiB per file | Rust core and C adapter | Reject the file |
| Decrypted incoming files | 50 MiB per Signal message | Rust backend | Reject remaining attachments |
| Admitted outgoing files | 2 files and 50 MiB total per account | Rust core | Retryable queue-full result |
| Queued binary events | 64 MiB aggregate | Rust event queue | Producer backpressure; a larger single event fails visibly and reconnects |
| Unresolved receive prompts | 64 MiB | C adapter | Ask the user to resolve existing prompts |
| Inline group JPEG/PNG | 8192 pixels per edge and 16 megapixels | C image decoder | Fall back to a file prompt |
| Inline group GIF | 8 MiB and 8 million cumulative canvas pixel-frames | C GIF parser and image decoder | Fall back to a file prompt |
| Signal GIF-style MP4 conversion | 8 MiB input/output, 480 pixels per edge, 15 fps maximum, 2 attempts per message, 1 process globally | Rust worker, `prlimit`, and FFmpeg | Preserve the original MP4 file prompt |

Incoming downloads use the smaller of the per-file limit and the message's
remaining byte budget. Presage stops the encrypted stream after the bounded
plaintext allowance plus Signal privacy padding and cryptographic framing, then
checks the decrypted length again. Missing or understated sender metadata
therefore cannot turn either limit into an unbounded allocation.

Signal GIF-style conversion additionally runs with a cleared environment,
pipe-only FFmpeg input and output, one codec thread, a 1 GiB address-space
limit, a 10-second soft and 12-second hard CPU limit, a 15-second wall limit,
and 64 file descriptors. Generated GIFs must pass the same structure, byte,
dimension, frame-area, and aggregate message-presentation budgets as native
GIFs before replacing the original presentation. No media temporary file is
created. Missing helpers, contention, process failure, timeout, invalid output,
or any exceeded budget leaves the downloaded MP4 unchanged.

Outgoing admission covers commands waiting in the core, commands deferred
during recovery, and active upload tasks. A non-cloneable permit owns both the
payload budget and request identifier through terminal event delivery; dropping
it on rejection, cancellation, panic, recovery failure, or shutdown restores
capacity automatically. Cancellation changes state directly on this permit,
without depending on bounded command-queue capacity, and the upload task
observes the same state before and during network work. The two-file admission
ceiling also bounds concurrent uploads.

The retained-payload limit is not a complete resident-memory bound. The C
adapter holds one bounded local-file copy while it crosses the FFI, and upstream
libraries may temporarily hold additional copies. The adapter opens the path
once, rejects non-regular and known-oversized inputs before allocating their
contents, and still stops after the limit plus one byte if the file grows.
Attachments are not stored in the durable text-message outbox and must be sent
again after a failed upload or restart.
