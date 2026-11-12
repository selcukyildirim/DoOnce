#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
ARCH="$(uname -m)"
NAME="doonce-airgap-${VERSION}-${ARCH}"
OUT="dist/${NAME}"

echo "Building DoOnce ${VERSION} for ${ARCH}"
cargo build --release -p doonce-cli
(
  cd web
  npm ci
  npm run build
)

rm -rf "$OUT"
mkdir -p "$OUT/bin" "$OUT/web"
cp target/release/doonce "$OUT/bin/doonce"
cp -R web/dist/* "$OUT/web/"
cp -R deploy "$OUT/deploy"
cp -R docs "$OUT/docs"
cp -R examples "$OUT/examples"
cp .env.example README.md SECURITY.md CONTRIBUTING.md LICENSE "$OUT/"

cat > "$OUT/INSTALL.txt" <<'INSTALL'
DoOnce air-gapped bundle
========================

1. Create a service account and directories:

   useradd --system --home /var/lib/doonce --shell /usr/sbin/nologin doonce
   install -d -o doonce -g doonce /var/lib/doonce /opt/doonce /etc/doonce

2. Install the binary and web console:

   install -m 0755 bin/doonce /usr/local/bin/doonce
   cp -R web /opt/doonce/web
   cp deploy/systemd/*.service /etc/systemd/system/

3. Copy your environment file to /etc/doonce/doonce.env and set at minimum:

   DOONCE_DEPLOYMENT_MODE=airgapped
   DOONCE_REGION=<your-region>
   DOONCE_HOME=/var/lib/doonce
   DOONCE_ARTIFACT_DIR=/var/lib/doonce/artifacts
   DOONCE_UI_DIR=/opt/doonce/web
   DOONCE_DATABASE_URL=postgres://...   # or sqlite:///var/lib/doonce/doonce.db?mode=rwc
   DOONCE_MASTER_KEY=<base64 32-byte key>
   DOONCE_CHROME=/usr/bin/chromium

4. Bootstrap the first tenant/API key:

   sudo -u doonce env $(cat /etc/doonce/doonce.env | xargs) doonce quickstart

5. Enable services:

   systemctl daemon-reload
   systemctl enable --now doonce.service doonce-worker.service

The bundle contains no external provider credentials and no telemetry. Use a
local Ollama or loopback OpenAI-compatible endpoint for AI features.
INSTALL

(
  cd dist
  find "$NAME" -type f -print0 | sort -z | xargs -0 sha256sum > "${NAME}.sha256"
  tar -czf "${NAME}.tar.gz" "$NAME"
  sha256sum "${NAME}.tar.gz" > "${NAME}.tar.gz.sha256"
)
echo "Created dist/${NAME}.tar.gz"
