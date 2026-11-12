# DoOnce TypeScript SDK

Dependency-free client for Node 18+ and modern browsers. It uses the global
`fetch` implementation (override it with `options.fetch` if needed).

```bash
npm install @doonce/sdk
```

```ts
import { DoOnceClient } from "@doonce/sdk";

const api = new DoOnceClient({
  baseUrl: process.env.DOONCE_API_URL,
  apiKey: process.env.DOONCE_API_KEY,
});

const definition = {
  schemaVersion: "2.0",
  name: "Example",
  blocks: [
    {
      id: "open",
      type: "browserAction",
      action: { kind: "navigate", url: "https://example.com/" },
      next: "done",
    },
    { id: "done", type: "terminate", status: "completed" },
  ],
};

const { workflow } = await api.createWorkflow(definition);
const { run } = await api.runWorkflow(workflow.slug, {});
const final = await api.waitForRun(run.id);

const { artifacts } = await api.listRunArtifacts(run.id);
for (const artifact of artifacts) {
  const bytes = await api.downloadArtifact(artifact.id);
  console.log(artifact.filename, bytes.byteLength);
}

console.log((await api.metrics()).metrics);
```

Errors are `DoOnceError` instances with `status`, `title`, `detail` and the raw
problem `body`. Use a scoped API key per integration.
