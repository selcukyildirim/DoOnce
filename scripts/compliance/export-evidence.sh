#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

COMMIT="$(git rev-parse HEAD 2>/dev/null || echo unknown)"
BRANCH="$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo unknown)"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
OUT="dist/evidence/doonce-evidence-${COMMIT:0:12}-${STAMP}"
mkdir -p "$OUT"

hash_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1"
  else
    shasum -a 256 "$1"
  fi
}

rustc --version > "$OUT/rustc.txt" 2>&1 || true
cargo --version > "$OUT/cargo.txt" 2>&1 || true
node --version > "$OUT/node.txt" 2>&1 || true
python3 --version > "$OUT/python.txt" 2>&1 || true
hash_file Cargo.lock > "$OUT/Cargo.lock.sha256"
while IFS= read -r -d '' migration; do
  hash_file "$migration"
done < <(find crates/doonce-store/migrations -type f -print0 | sort -z) > "$OUT/migrations.sha256"
cargo metadata --no-deps --format-version 1 > "$OUT/cargo-metadata.json" 2>/dev/null || true

if command -v cargo-deny >/dev/null 2>&1; then
  cargo deny check > "$OUT/cargo-deny.txt" 2>&1 || true
fi
if [ -n "${TEST_LOG:-}" ] && [ -f "$TEST_LOG" ]; then
  cp "$TEST_LOG" "$OUT/tests.txt"
fi
if command -v cargo-cyclonedx >/dev/null 2>&1; then
  cargo cyclonedx --format json --output-file "$OUT/doonce.cdx.json" >/dev/null 2>&1 || true
fi

python3 - "$OUT" "$COMMIT" "$BRANCH" <<'PY'
import json, pathlib, sys
out, commit, branch = sys.argv[1], sys.argv[2], sys.argv[3]
files = {}
for path in sorted(pathlib.Path(out).glob("*")):
    if path.name == "evidence.json":
        continue
    files[path.name] = path.stat().st_size
payload = {
    "product": "doonce",
    "commit": commit,
    "branch": branch,
    "generatedAt": __import__("datetime").datetime.now(__import__("datetime").timezone.utc).isoformat(),
    "files": files,
    "notes": "Technical evidence only. External audit, penetration test, DR drill and legal agreements are separate gates.",
}
pathlib.Path(out, "evidence.json").write_text(json.dumps(payload, indent=2) + "\n")
PY

echo "Evidence written to $OUT"
