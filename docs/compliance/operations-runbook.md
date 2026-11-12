# Operations, incident and DR runbook

## Incident response

1. **Detect** via Prometheus alerts, health checks, audit anomalies or a report
   to the security contact.
2. **Triage** severity (Critical/High/Medium/Low) and open a private incident
   record. Preserve logs, audit hashes and relevant artifact metadata.
3. **Contain** by revoking API/connector tokens, disabling a tenant/connector,
   blocking egress or scaling down a compromised worker.
4. **Eradicate/recover** by rotating `DOONCE_MASTER_KEY` with re-encryption,
   restoring from backup, or redeploying a clean image.
5. **Notify** per contractual/legal timelines (GDPR/KVKK 72h where applicable).
6. **Postmortem** within 5 business days for High/Critical: timeline, impact,
   root cause, corrective actions and evidence links.

## Backup

- Postgres: use managed PITR or `pg_dump --format=custom` on a schedule.
- SQLite single-node: stop writes or use `VACUUM INTO`, then copy the file.
- Artifacts: snapshot `DOONCE_ARTIFACT_DIR` with the database backup so
  artifact rows and files stay consistent.
- Encrypt backups with a KMS-backed key separate from `DOONCE_MASTER_KEY` when
  possible; test restore access quarterly.

## Restore / DR

1. Provision a clean database and restore the latest backup.
2. Restore artifacts to the same relative paths under `DOONCE_ARTIFACT_DIR`.
3. Provide the same `DOONCE_MASTER_KEY` (or complete re-encryption migration)
   before starting the API/worker.
4. Start the API, verify `/api/v1/health`, `/api/v1/readyz`, audit-chain
   verification and a small workflow.
5. Record RTO/RPO, backup timestamp, restore duration and operator in the DR
   evidence log. Phase 11 DR drills are still required before GA.

## Key rotation

- API keys: create the replacement, deploy it, revoke the old key.
- Connector tokens: delete/recreate the connector; queued jobs fail closed.
- `DOONCE_MASTER_KEY`: requires a planned re-encryption migration; do not swap
  the key in place without migrating credentials/IdP secrets/webhook secrets.
