# Compliance Control Matrix

This matrix maps the target frameworks to technical controls and the evidence
we will produce. It is a living document; certification also requires
organizational policies, training, vendor management, audits and legal
agreements.

Legend: **Implemented** = code + tests in this repository. **Partial** =
foundation exists, production control still pending. **Planned** = scheduled.

## SOC 2 Type II / ISO 27001

| Control area | Technical implementation | Evidence | Status |
|---|---|---|---|
| Logical access | API keys (Argon2), RBAC, tenant scoping; SSO/SCIM/MFA target | Auth tests, access review export, RBAC tests | Partial |
| Change management | PR review, CI gates, reproducible release workflow, SBOM/provenance/cosign, migration policy | Git history, CI logs, release attestations | Partial |
| Encryption | AES-256-GCM, KMS/HSM target, TLS 1.3; connector mTLS target | Crypto tests, KMS config, TLS scan | Partial |
| Audit logging | Hash-chained audit log, SIEM export target | `audit_logs`, chain verification tests | Partial |
| Availability | SLOs, multi-AZ/DR targets, backups | SLO dashboards, DR drill reports | Planned |
| Incident response | Severity matrix, on-call, postmortems | Runbooks, incident tickets | Planned |
| Vendor management | LLM/storage/email subprocessor register, DPAs | Subprocessor list, contracts | Planned |
| Vulnerability management | cargo-audit/deny, release SBOM, dependency updates, DAST target | CI reports, SBOM, remediation tickets | Partial |
| Secure SDLC | Threat model, ADR, security review, fuzz/property tests | ADRs, review records | Partial |
| Asset inventory | Workspace crates, deploy images, data stores | Source SBOM in release CI, deployment manifests | Partial |

## GDPR / KVKK

| Requirement | Technical implementation | Evidence | Status |
|---|---|---|---|
| Lawful basis and records | Tenant data map, purpose tags | ROPA, DPIA | Planned |
| Data minimization | Secrets/PHI not stored by default; artifact retention | Config + retention jobs | Partial |
| Right of access / portability | Tenant data export API | Export job logs | Planned |
| Right to erasure | Tenant-scoped cascading delete + artifact purge | Deletion tests, audit | Partial |
| Breach notification | Incident runbook, 72h process | Incident records, notification templates | Planned |
| Data residency | Tenant/connector region pinning, per-installation region, local artifacts in data plane | `DOONCE_REGION`, tenant/connector rows, deployment manifests | Partial |
| Subprocessors / transfers | Vendor register, SCCs, KVKK transfer records | Contracts, register | Planned |
| Security of processing | Encryption, access control, audit | Same as SOC 2 | Partial |

## HIPAA

| Requirement | Technical implementation | Evidence | Status |
|---|---|---|---|
| BAA | Contractual process with customers/subprocessors | Signed BAAs | Planned |
| PHI minimization | PHI excluded by default; dedicated data plane/region for PHI tenants | Tenant configuration, connector deployment, data-flow diagram | Partial |
| Access control | RBAC, MFA/SSO, unique IDs, emergency access | Access reviews | Partial |
| Audit controls | Hash-chained audit, SIEM export | Audit reports | Partial |
| Integrity | Hash chain, backups, checksums | Verification reports | Partial |
| Transmission security | TLS 1.3, mTLS | TLS scan | Partial |
| Contingency plan | Backup, DR, RTO/RPO drills | Drill reports | Planned |
| Training / sanctions | HR process, LMS evidence | Training records | Planned |

## PCI-DSS

**Scope decision:** DoOnce does not store, process or transmit cardholder data.
Payment flows must use the payment provider's hosted fields/tokenization. This
keeps the assessment in SAQ-A/A-EP territory. If a customer routes card data
through browser automation, that customer deployment becomes in-scope and
requires a separate review; the default product must reject storing PAN/CVV.

| Requirement | Technical implementation | Evidence | Status |
|---|---|---|---|
| Scope reduction | No PAN/CVV storage; no card fields in artifacts by default | Data-flow diagram, schema review | Partial |
| Network segmentation | Outbound-only connector leases, separate data-plane artifacts/credentials, Helm/network controls | Connector tests, deployment manifests | Partial |
| Encryption | TLS 1.3, AES-256-GCM, KMS/HSM | Config + scan | Partial |
| Access control | RBAC, MFA/SSO, least privilege | Access review | Partial |
| Logging | Audit log, SIEM export | Audit reports | Partial |
| Vulnerability scans | ASV scan + pen test | ASV reports | Planned |
| Secure development | Secure SDLC, code review, dependency policy | CI + ADRs | Partial |

## Evidence automation (planned)

- Policy-as-code checks in CI (branch protection, required reviews, signed
  commits optional).
- Automated control evidence export (JSON/CSV) for auditors.
- Access-review snapshots per quarter.
- Vulnerability SLA dashboard.
- SBOM + provenance per release.
- DR restore attestations.
- Data-retention and deletion job reports.
