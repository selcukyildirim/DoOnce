# ADR 0010 — Block engine: control flow, HTTP, code and file parsing

- Status: Accepted
- Date: 2026-11-06

## Context

Workflows need more than linear browser actions. The engine must support loops,
conditionals, external HTTP calls, small deterministic code steps and local file
parsing while remaining safe for untrusted imported workflows.

## Decision

1. **Control flow lives in the executor.** `forLoop`, `conditional` and `goto`
   are handled by the workflow loop, not by recursive sub-workflow calls.
2. **Loop contract:** the body starts at `body`; each iteration ends when the
   body reaches the `forLoop` block again (via `next`/`goto`). `maxIterations`
   caps the list, and a global step budget prevents runaway execution.
3. **Conditions use JSON Logic** evaluated against a nested view of the run
   scope and block outputs (`outputs.<block>...`). Missing/undefined values are
   false rather than a hard error.
4. **HTTP blocks are guarded.** Only http(s); private/loopback/link-local/
   metadata addresses are rejected by validation and again at runtime after DNS
   resolution. Redirects are disabled. Request headers `Host`,
   `Content-Length`, `Transfer-Encoding` and `Connection` are rejected.
   Responses are capped at 2 MB, non-2xx fails the block.
   `allow_private_http` is an explicit opt-in for local integrations/tests.
5. **Code blocks are sandboxed JavaScript** (Boa): no host functions, no
   filesystem/network access, loop/recursion limits and a wall-clock timeout.
   Python code blocks are rejected in this build.
6. **File parsing is confined to `artifact_dir`.** Paths are canonicalized and
   must remain under the configured directory. JSON and CSV are supported;
   XLSX/XML/PDF return `unsupported` until their parsers land.
7. **Outputs are explicit.** Leaf blocks record `outputs[blockId]` and flatten
   to `blockId.output.*`; `saveAs` also records a named variable. Array index
   keys are not flattened because that would turn arrays into objects.
8. **AI, email, upload, download-files, login and sub-workflow blocks remain
   unsupported** in this phase and fail closed.

## Consequences

- Deterministic workflows can now cover most RPA-shaped tasks without an LLM.
- Loop semantics are explicit and auditable; malformed loop bodies fail rather
  than silently running once.
- The HTTP/code/file surfaces are new attack surfaces and are covered by the
  threat model and dedicated tests.
- **Known advisory:** Boa pulls in the unmaintained `paste` proc-macro
  (RUSTSEC-2024-0436). It is build-time only, not runtime-reachable, and tracked
  in `deny.toml`.
- Phase 5b/6 will add AI blocks, email, object storage, login and sub-workflows
  using the same execution and approval model.
