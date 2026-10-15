#!/usr/bin/env bash
# Builds doonce and registers the native messaging host for Chrome.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
source "${HOME}/.cargo/env" 2>/dev/null || true
# Cursor/sandbox may set CARGO_TARGET_DIR; the host manifest must point at a stable path.
unset CARGO_TARGET_DIR
cargo build --release -p doonce-cli
BIN="$ROOT/target/release/doonce"
if [[ ! -x "$BIN" ]]; then
  echo "error: expected binary at $BIN" >&2
  exit 1
fi
"$BIN" setup
echo ""
echo "Next:"
echo "  1. cd extension && npm run build"
echo "  2. Chrome → Extensions → Load unpacked → $ROOT/extension/dist"
echo "  3. Click the DoOnce icon → side panel should open"
