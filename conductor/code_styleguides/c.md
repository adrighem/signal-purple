# C Style

- Follow the existing four-space indentation and GLib ownership conventions.
- Use `g_autoptr` and `g_autofree` where ownership is local and unambiguous.
- Keep every `PurpleXfer`, request, source, and connection callback detached
  before freeing its owning connection.
- Check every fallible Rust ABI result when it affects user-visible state.
- Use bounded, single-open file I/O for attachment admission.
