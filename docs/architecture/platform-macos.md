# macOS Platform Layer

macOS uses the shared VT runtime with AppKit-specific host integration.

Primary files:

- `src/macos_vt_backend.rs`
- `src/platform/macos.rs`
- `src/keymap.rs`

Platform-specific responsibilities:

- host views and focus
- AppKit text input and IME integration
- clipboard and notifications
- macOS event plumbing

The design goal is to keep terminal state and session logic out of the
platform-specific layer whenever possible.
