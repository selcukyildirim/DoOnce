# Deployment guide

DoOnce runs as a control plane (API/UI/scheduler metadata) and one or more data
planes (API/worker/browser/artifacts). The default single-node install runs both
planes together; the hybrid connector lets a customer VPC data plane connect
outbound-only to a SaaS control plane.

## Modes

| Mode | Use |
|---|---|
| `saas` | Multi-tenant hosted control plane with remote data planes |
| `onprem` | Customer VPC/self-hosted; may use the hybrid connector |
| `airgapped` | No external network; telemetry is refused and AI must be local |

Set `DOONCE_DEPLOYMENT_MODE`, `DOONCE_REGION`, `DOONCE_HOME`,
`DOONCE_ARTIFACT_DIR`, `DOONCE_DATABASE_URL` and `DOONCE_MASTER_KEY`. In
air-gapped mode `--dev-no-auth` and telemetry are rejected at startup.

## Docker Compose (single node)

```bash
cp .env.example .env
# set POSTGRES_PASSWORD, DOONCE_MASTER_KEY, LLM/provider variables
docker compose up -d --build
curl http://127.0.0.1:8080/api/v1/health
```

The API and worker share the `doonce-data` volume so the worker can write
screenshots and the API can serve them. The image runs as UID 10001 and uses a
Chromium wrapper with `--no-sandbox` for containerized operation. For
production, run browsers in a separate sandboxed data plane (gVisor, Kata or a
dedicated VM).

## Helm

```bash
kubectl create namespace doonce
kubectl -n doonce create secret generic doonce-secrets \
  --from-literal=DOONCE_DATABASE_URL='postgres://doonce:pass@postgres:5432/doonce' \
  --from-literal=DOONCE_MASTER_KEY='<base64 32-byte key>'

helm upgrade --install doonce deploy/helm/doonce -n doonce \
  --set image.tag=0.1.0 \
  --set deploymentMode=onprem \
  --set region=eu-central
```

For an external Postgres, omit the bundled database dependency and provide the
secret. The chart creates the API Deployment/Service, worker Deployment, shared
PVC, ConfigMap, ServiceAccount and optional Ingress. The connector Deployment is
disabled by default.

## systemd

Install `deploy/systemd/doonce.service`, `doonce-worker.service` and optionally
`doonce-connector.service`. Keep environment files at `/etc/doonce/doonce.env`
(control plane) and `/etc/doonce/connector.env` (connector). The units drop all
capabilities, use `NoNewPrivileges`, `PrivateTmp`, `ProtectSystem=strict` and
write only under `/var/lib/doonce`.

## Hybrid connector

The connector is outbound-only and has no inbound listener:

1. On the control plane, create a connector and copy the one-time token:

   ```bash
   curl -sS -X POST "$CONTROL/api/v1/connectors" \
     -H "x-api-key: $ADMIN_KEY" -H 'content-type: application/json' \
     -d '{"name":"customer-vpc","region":"eu-central"}'
   ```

2. Bind the tenant to that connector (fail-closed if the connector is deleted):

   ```bash
   curl -sS -X PUT "$CONTROL/api/v1/settings" \
     -H "x-api-key: $ADMIN_KEY" -H 'content-type: application/json' \
     -d '{"key":"execution.connector_id","value":"<connector-id>"}'
   ```

3. Start the data plane locally (`doonce serve` + `doonce worker`) and run the
   connector on the same network:

   ```bash
   DOONCE_CONTROL_URL=https://doonce.example.com \
   DOONCE_CONNECTOR_TOKEN=dsk_... \
   DOONCE_CONNECTOR_LOCAL_URL=http://127.0.0.1:8080 \
   DOONCE_CONNECTOR_LOCAL_API_KEY=dsk_... \
   doonce connector --auto-approve
   ```

The connector claims queued runs/tasks, upserts the workflow into the local
data plane, waits for the local worker, approves local approval gates only with
`--auto-approve`, and reports status/output back. Screenshots and credentials
stay local; the control plane receives artifact metadata only. Local artifact
downloads are performed on the data plane. Workflows that reference cloud-only
credentials should use external credential providers or local equivalents.

## Data residency

- `DOONCE_REGION` pins the installation; tenant/connector regions are stored and
  connectors must match the tenant region.
- Artifacts and browser profiles live under `DOONCE_ARTIFACT_DIR` in the data
  plane, never in the SaaS control plane when using a connector.
- Use one control plane/data plane pair per residency boundary. The single
  database is not a multi-region router; run separate installations for
  separate legal regions.

## Air-gapped install

```bash
./deploy/scripts/build-airgap-bundle.sh
sha256sum -c dist/doonce-airgap-*.tar.gz.sha256
tar -xzf dist/doonce-airgap-*.tar.gz
sudo ./doonce-airgap-*/INSTALL.txt
```

The bundle includes the release binary, web console, systemd units, docs and
examples. Configure a local Ollama or loopback OpenAI-compatible endpoint; the
worker rejects non-loopback LLM endpoints in air-gapped mode.
