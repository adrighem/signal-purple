# Attachment resource policy

These are local defensive limits, not Signal service guarantees. They bound
different ownership stages independently and may change without a C ABI bump.

| Direction and stage | Limit | Owner | Outcome |
| --- | ---: | --- | --- |
| Incoming or outgoing file | 25 MiB per file | Rust core and C adapter | Reject the file |
| Decrypted incoming files | 50 MiB per Signal message | Rust backend | Reject remaining attachments |
| Admitted outgoing files | 2 files and 50 MiB total per account | Rust core | Retryable queue-full result |
| Queued binary events | 64 MiB | Rust event queue | Visible overflow and reconnect |
| Unresolved receive prompts | 64 MiB | C adapter | Ask the user to resolve existing prompts |
| Inline group images | 8192 pixels per edge and 16 megapixels | C image decoder | Fall back to a file prompt |

Incoming downloads use the smaller of the per-file limit and the message's
remaining byte budget. Presage stops the encrypted stream after the bounded
plaintext allowance plus Signal privacy padding and cryptographic framing, then
checks the decrypted length again. Missing or understated sender metadata
therefore cannot turn either limit into an unbounded allocation.

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
