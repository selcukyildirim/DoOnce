# ADR 0011 — Service-backed blocks: AI, email, upload, login, nested workflows

- Status: Accepted
- Date: 2026-11-06

## Context

Phase 5a made the deterministic block engine usable. Browser automation workflows
also need AI blocks, email, file upload, credentials/login, file downloads and
sub-workflows. Putting provider SDKs and database access directly into the
engine would create cycles (`agent -> engine`) and make the core impossible to
test without infrastructure.

## Decision

1. **Engine defines service traits, not implementations.** `AiExecutor`,
   `CredentialResolver`, `EmailSender`, `ArtifactStore` and `WorkflowResolver`
   live in `doonce-engine`; the engine calls only these traits.
2. **Concrete services live in `doonce-services`.** That crate depends on the
   agent, LLM client, store and security crates and provides:
   - `AiService`: agent tasks, extract/validate/prompt, text prompts and file
     downloads with vision.
   - `CredentialService`: DB vault decryption + TOTP code generation.
   - `SmtpEmailService`: SMTP via `lettre` (`DOONCE_SMTP_URL`,
     `DOONCE_EMAIL_FROM`).
   - `LocalArtifactStore`: copy artifacts into a restricted upload root.
   - `DbWorkflowResolver`: load nested workflows from tenant storage.
3. **Approval flows through the engine.** Agent actions still call the engine's
   approval callback, so risky browserTask actions keep the same approval UX as
   deterministic steps.
4. **BrowserTask uses the agent.** `browserTask` passes prompt/URL/schema/error
   codes/max steps to the agent and records its output.
5. **AI extract/validate/textPrompt** call the agent/LLM directly and record a
   structured output. Validation failures remain typed failures.
6. **Login is deterministic and heuristic.** It resolves a vault credential,
   fills common username/email and password fields, optionally fills a TOTP
   field, and submits with Enter. AI-assisted login remains a future
   enhancement; password-manager adapters are Phase 6.
7. **Nested workflows** are loaded by the resolver and executed with the same
   browser, observer, services and options. `Services.max_depth` (default 3)
   prevents recursion.
8. **Upload** copies to a canonicalized local upload root; path traversal is
   rejected. S3/GCS/Azure are future implementations of the same trait.
9. **Missing services fail closed.** If AI/email/credentials/workflows are not
   configured, the block returns `unsupported`/action failure with a clear
   message; nothing is silently skipped.

## Consequences

- The engine remains testable with mocks; services are tested independently.
- The CLI wires real services from the DB + environment when available and
  falls back to local-only services otherwise.
- SMTP credentials are read from the SMTP URL/environment, never stored in a
  workflow.
- Login heuristics can be extended without changing workflow files.
- Phase 6/7 can add password managers, object storage and a worker using the
  same traits.
