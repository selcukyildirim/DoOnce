# ADR 0002 — Clean-room licensing and AGPL boundary

- Status: Accepted
- Date: 2026-11-06

## Context

DoOnce is Apache-2.0. The product must remain commercially usable without
forcing customers to disclose their own integrations or hosted modifications.
Copying source, prompts, schemas or generated assets from AGPL/GPL projects
into this repository would create a derivative work and require copyleft
compliance.

## Decision

- Treat any AGPL/GPL project as a **behavioral and public-API reference only**.
- Do not copy source code, prompt text, proprietary schemas, images or test
  fixtures from copyleft projects.
- Do not vendor copyleft projects as a runtime dependency.
- Reimplement features from first principles using public documentation,
  observable behavior and our own design.
- Keep a provenance log for any external reference used during design.
- Reuse only code whose license is compatible with Apache-2.0.

## Enforcement

- Code review asks: "Was this written from our design, or copied?"
- CI includes `cargo-deny` with AGPL/GPL denied.
- Contributions must include a DCO/sign-off confirming provenance.
- If a business decision later requires copyleft code, the entire affected
  component must be isolated under its original license and the licensing model
  revisited with counsel. It cannot be mixed into the Apache core.

## Consequences

- Some features take longer to build.
- The codebase remains safe for enterprise procurement and commercial
  distribution under Apache-2.0.
