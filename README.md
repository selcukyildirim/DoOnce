# DoOnce

**Teach Once. Run Forever. Fix Itself.**

DoOnce records what you do in the browser once, turns it into a reusable workflow, and replays it deterministically—without calling an LLM on every run. When a site changes and a step breaks, a local AI CLI can propose a repair (coming in Phase 2).

## Quick start

### 1. Build the Rust CLI

```bash
cargo build --release
```

Binary: `target/release/doonce`

### 2. Connect Chrome

```bash
./scripts/install-host.sh
# or: cargo build --release -p doonce-cli && ./target/release/doonce setup
```

Then load **`extension/dist`** (not the `extension/` folder). Reload the extension after setup.

If the side panel is blank, rebuild the extension (`npm run build` in `extension/`) — asset paths must be relative.

### 3. Build the extension

```bash
cd extension
npm install
npm run build
```

In Chrome: **Extensions → Load unpacked →** choose `extension/dist`.

### 4. Try the demo site

```bash
doonce demo
# open http://127.0.0.1:4317/
```

Record from the DoOnce side panel, or import a sample workflow:

```bash
doonce import examples/workflows/contact-demo.json
doonce run contact-demo
```

## Architecture (Phase 1)

| Piece | Role |
|--------|------|
| **Chrome extension** | Record, list, run; drives your existing Chrome profile |
| **Native messaging** | Extension ↔ `doonce` host (no open TCP port) |
| **Rust engine** | Workflow replay, SQLite storage, CDP driver for CLI runs |
| **page-agent.js** | Same locator/action code in extension and CDP |

## Data & privacy

- Workflows live in SQLite under `~/Library/Application Support/DoOnce` (macOS) or `$DOONCE_HOME`.
- Passwords and similar fields are **never** stored in recordings—only `{{secret}}` parameters.
- Exported workflows are sanitized (no session tokens in URLs).

## Tests

```bash
cargo test
cargo test -p doonce-engine --test demo_replay   # needs Chrome
cd extension && npm test
```

## License

Apache-2.0 — see [LICENSE](LICENSE).
