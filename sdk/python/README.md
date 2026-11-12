# DoOnce Python SDK

Dependency-free synchronous client for the DoOnce control plane. Requires
Python 3.9+.

```bash
pip install -e sdk/python
```

```python
import os
from doonce import DoOnce

api = DoOnce(
    base_url=os.environ.get("DOONCE_API_URL", "http://127.0.0.1:8080"),
    api_key=os.environ["DOONCE_API_KEY"],
)

workflow = {
    "schemaVersion": "2.0",
    "name": "Example",
    "blocks": [
        {
            "id": "open",
            "type": "browserAction",
            "action": {"kind": "navigate", "url": "https://example.com/"},
            "next": "done",
        },
        {"id": "done", "type": "terminate", "status": "completed"},
    ],
}

slug = api.create_workflow(workflow)["workflow"]["slug"]
run = api.run_workflow(slug)["run"]
final = api.wait_for_run(run["id"], timeout=300)
print(final["status"])

for artifact in api.list_run_artifacts(run["id"])["artifacts"]:
    api.download_artifact(artifact["id"], output=f"{artifact['id']}.png")

print(api.metrics()["metrics"])
```

The client raises `DoOnceError` with `status`, `title`, `detail` and the raw
problem body. No credentials are logged. Use a scoped API key per integration.
