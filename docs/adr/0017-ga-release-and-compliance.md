# ADR 0017 — GA release engineering and compliance evidence

- Status: Accepted
- Date: 2026-11-07

## Context

The technical controls were implemented across Phases 1–10, but a GA candidate
also needs reproducible builds, an SBOM/signature trail, production guardrails,
operational runbooks and a clear boundary between engineering evidence and an
external audit. Phase 11 (pen-test/DR/live evidence automation) was explicitly
skipped by request.

## Decision

1. **Production fails closed.** `DOONCE_ENV=production` requires Postgres, a
   KMS/secret-manager `DOONCE_MASTER_KEY`, an HTTPS public URL, secure cookies
   and no dev-no-auth. SQLite is rejected after connection, before serving.
2. **Reproducible tagged releases.** `.github/workflows/release.yml` builds
   Linux/macOS/Windows binaries, publishes checksums, a source SPDX SBOM, a
   GHCR image with BuildKit provenance/SBOM and a cosign keyless signature.
3. **Evidence exporter.** `scripts/compliance/export-evidence.sh` captures
   commit metadata, tool versions, lockfile/migration hashes, cargo-deny output,
   test logs when supplied and an optional CycloneDX SBOM. It explicitly marks
   external audits as separate gates.
4. **Runbooks over claims.** `docs/compliance/` contains incident, backup/DR,
   access-review, vendor and retention procedures. `docs/GA-CHECKLIST.md`
   distinguishes code-complete controls from pen-test, DR drills and audits.
5. **No certification claim.** SOC 2 / ISO 27001 / HIPAA / PCI programs require
   external auditors, legal agreements and organizational evidence. This
   repository provides technical controls and hooks only.

## Consequences

- Releases are verifiable and rollbackable; CI becomes the source of build and
  dependency evidence.
- Phase 11 remains a release blocker for a production GA announcement even
  though engineering controls are complete.
- Rotation of the master key still requires a planned re-encryption migration;
  the runbook calls this out rather than pretending a key swap is transparent.
