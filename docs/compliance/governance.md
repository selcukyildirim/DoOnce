# Governance: access, vendors, retention

## Access review

- Quarterly review of users, roles, API keys, connector tokens and IdP mappings.
- Remove departed users via SCIM deprovisioning or suspend the account.
- Require MFA for Owners/Admins; review break-glass access after every use.
- Export `/api/v1/audit` and access lists as evidence; store with the release or
  audit bundle.

## Vendor / subprocessor register

Maintain owner, purpose, data categories, region, DPA/SCC status and review date
for at least: cloud hosting, Postgres, object storage, LLM provider, email/SMTP,
credential manager and observability. Air-gapped customers must have a
provider-free configuration or an approved local equivalent (for example
Ollama).

## Data retention

- Workflow/run/task metadata: customer-defined retention, default 90 days after
  terminal state.
- Artifacts/screenshots: default 30 days; PHI/cardholder data must not be
  captured. Artifact deletion must remove both the file and the row.
- Audit logs: minimum 1 year (or contractual/legal requirement), immutable
  export to SIEM/WORM storage.
- Backups: documented retention and deletion schedule; legal hold overrides.
- Erasure requests: tenant-scoped cascade plus artifact purge and backup expiry.
