# ADR 001: Extension ↔ Rust via Native Messaging

## Status

Accepted (Phase 1)

## Context

DoOnce needs the Chrome extension and the Rust workflow engine to share one SQLite database and one replay implementation. Options:

1. **Chrome Native Messaging** — Chrome spawns `doonce` as a child process; length-prefixed JSON on stdin/stdout.
2. **Localhost HTTP/WebSocket** — a daemon listens on a port.

## Decision

Use **Native Messaging** for the extension path.

- No background server or port for users to configure.
- Chrome enforces `allowed_origins` in the host manifest.
- The same `doonce` binary serves CLI, CDP replay, and the native host (`chrome-extension://…` as argv).

Browser automation during replay uses **extension APIs** (`scripting`, `tabs`, `downloads`) in the user’s normal Chrome. CLI replay uses **CDP** with a dedicated profile under `DoOnce/chrome-profile`.

## Consequences

- Users run `doonce setup` once per browser profile.
- Extension id is fixed via manifest `key` for stable host registration.
- Driver calls from Rust are forwarded to the extension (`navigate`, `call`, `perform`, …).
