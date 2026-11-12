# ADR 0006 — Deterministic locator and replay semantics

- Status: Accepted
- Date: 2026-11-06

## Context

Browser automation breaks when locators rely on a single fragile signal (an
XPath, a generated CSS class or an index). DoOnce must replay recorded actions
reliably, produce explainable failures and avoid unsafe retries.

## Decision

1. **Multi-signal scoring.** A target is matched by combining role, accessible
   name, label, visible text, stable attributes (`data-testid`, `id`, `name`,
   `aria-label`, `placeholder`, `href`, ...), context (nearby heading/landmark)
   and candidate CSS selectors. A unique selector is strong evidence, but never
   the only signal required when more are present.
2. **Ambiguity is an error.** If two visible candidates are within a score
   margin, replay must not guess. The engine reports `ambiguous` with
   diagnostics. A single pointer-hit-testable candidate may break the tie.
3. **Isolated world.** The page agent is evaluated in a Chrome isolated world.
   Page scripts cannot override locator or action code.
4. **No XPath and no generated selectors.** Avoid CSS-in-JS hashes, React ids
   and long random class names. Structural paths are a last resort.
5. **Fail closed.** Unsupported block types, missing targets, disabled controls,
   hidden elements and failed conditions stop the run with a typed failure.
   Nothing is silently skipped.
6. **Retry only when safe.** Actions that may already have reached the page
   (`dispatched`) are not retried automatically. Idempotent navigation and
   waiting can be retried per block configuration.
7. **Human approval at the executor boundary.** Risk classification runs before
   the block executes; the observer must approve, and denial is a cancellation,
   not a silent skip.
8. **Deterministic ordering.** Blocks execute by `next`, then array order;
   `goto` provides explicit jumps. A step budget prevents infinite loops.

## Consequences

- Recordings need richer target metadata than one selector. This is a feature,
  not overhead: diagnostics explain exactly why a locator failed.
- The page agent is larger than a naive `querySelector` helper and must be
  reviewed like security-sensitive code.
- AI repair (Phase 3) can operate on the same `Target` schema and diagnostics.
- Future Playwright-compatible SDKs can map `Target` to their own locators
  without changing the workflow format.
