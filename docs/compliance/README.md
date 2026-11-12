# Compliance operations

DoOnce provides technical controls and evidence hooks; certification also
requires policies, training, vendor contracts, audits and customer agreements.
This directory contains the operational runbooks referenced by the control
matrix.

## Frameworks

- SOC 2 Type II / ISO 27001: access, change management, encryption, audit,
  availability, incident response, vendor management, vulnerability management.
- GDPR / KVKK: data map, purpose limitation, access/portability/erasure,
  residency, subprocessors and breach notification.
- HIPAA: BAA process, PHI minimization, access/audit/integrity/transmission
  controls, contingency plan.
- PCI-DSS: no PAN/CVV storage; hosted payment fields/tokenization only.

## Evidence

Run `scripts/compliance/export-evidence.sh` in CI or a release job. It writes
commit metadata, tool versions, dependency policy output, migration hashes and
an optional SBOM under `dist/evidence/`. Attach the bundle to the release or
audit request. Never include secrets, customer data or production URLs.

## Status language

Use **implemented** only for code + tests in this repository. Use **partial**
when technical controls exist but organizational evidence or an external audit
is missing. Use **planned** for work that has not been delivered. Do not claim
SOC 2, ISO 27001, HIPAA or PCI certification from this repository alone.
