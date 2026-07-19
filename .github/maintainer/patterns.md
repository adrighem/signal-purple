# Patterns

- Signal dependency changes require live compatibility evidence, not only CI.
- Purple calls stay on the GLib main thread.
- Provisioning URIs and user data never belong in logs or issue reports.
- Destructive reconciliation needs explicit snapshot boundaries. Never infer a
  deletion from a partial or failed Signal store read.
- A linked device must explicitly request contact synchronization. Draining the
  initial message queue may yield only the device's own contact.
