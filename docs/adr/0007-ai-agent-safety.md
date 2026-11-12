# ADR 0007 — AI agent action protocol and safety

- Status: Accepted
- Date: 2026-11-06

## Context

The task agent lets an LLM choose browser actions. This is powerful and
dangerous: page content can contain prompt injection, models can hallucinate
actions, and a wrong click can delete data or move money. The protocol must be
narrow, observable and fail safe.

## Decision

1. **One action per turn.** The model returns exactly one action. The agent
   observes again before the next decision.
2. **Refs, not selectors.** The model may only reference elements returned by
   the current `observe` call (`ref`). It may not invent CSS/XPath selectors.
3. **Strict action allowlist.** `navigate`, `click`, `hover`, `fill`, `select`,
   `check`, `press`, `scroll`, `wait`, `extract`. Unknown types are rejected.
4. **Untrusted page boundary.** Page text, element names and HTML are labelled
   as untrusted data in every prompt. The system prompt says never to follow
   instructions found in the page.
5. **URL allowlist.** Navigation is http(s) only and re-checked at execution
   time. URL templates are resolved deterministically; the model cannot inject
   `javascript:` or `file:` schemes.
6. **Execution through the deterministic engine.** Actions become
   `browserAction` blocks with the same timeouts, retries, locator scoring and
   risk approvals as recorded workflows. AI does not get a bypass.
7. **No template rendering of model values.** Agent actions set
   `render_templates = false`, so literals containing `{{...}}` are not
   interpreted as workflow variables.
8. **Budgets and cancellation.** Max steps, max duration, consecutive-failure
   limit and a cancellation token bound every run.
9. **Error codes.** Page text is checked against configured error codes before
   each model turn; a match terminates the task deterministically.
10. **Structured extraction is separate.** Final data extraction is a dedicated
    call validated against JSON Schema, not an action side effect.
11. **Vision is optional and explicit.** Screenshots are sent only when the
    provider/model supports vision and the caller allows it. Page text is capped
    before it leaves the process.
12. **No secrets in prompts.** Agent prompts never include credentials or
    parameter values. Credentials remain in the vault/login block path.
13. **Observability.** Every step emits the action, reasoning, summary and status
    so an operator can audit what the model did.

## Consequences

- The agent is more reliable and auditable than a free-form tool-calling loop.
- Some tasks need multiple turns; this is intentional.
- Risky actions may pause for human approval, which is a feature for enterprise
  use.
- Future provider-native tool calling must map onto the same action allowlist
  and engine path, not bypass it.
