import assert from "node:assert/strict";
import { createServer } from "node:http";
import { after, before, test } from "node:test";

import { DoOnceClient, DoOnceError } from "../dist/index.js";

let server;
let baseUrl;
let seen = [];

before(async () => {
  server = createServer((request, response) => {
    let body = "";
    request.on("data", (chunk) => (body += chunk));
    request.on("end", () => {
      seen.push({
        method: request.method,
        url: request.url,
        apiKey: request.headers["x-api-key"],
        body: body ? JSON.parse(body) : undefined,
      });
      const send = (status, payload, type = "application/json") => {
        const data = Buffer.isBuffer(payload)
          ? payload
          : typeof payload === "string"
            ? payload
            : JSON.stringify(payload);
        response.writeHead(status, { "content-type": type, "content-length": Buffer.byteLength(data) });
        response.end(data);
      };
      if (request.method === "GET" && request.url === "/api/v1/health") {
        send(200, { status: "ok" });
      } else if (request.method === "GET" && request.url.startsWith("/api/v1/workflows")) {
        send(200, { workflows: [{ slug: "demo" }] });
      } else if (request.method === "POST" && request.url === "/api/v1/workflows") {
        send(201, { workflow: { slug: "demo" } });
      } else if (request.method === "POST" && request.url === "/api/v1/workflows/demo/runs") {
        send(202, { run: { id: "r1", status: "queued" } });
      } else if (request.method === "GET" && request.url === "/api/v1/artifacts/a1") {
        send(200, Buffer.from([0x89, 0x50, 0x4e, 0x47]), "image/png");
      } else {
        send(404, { title: "Not found", detail: "missing" });
      }
    });
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  baseUrl = `http://127.0.0.1:${server.address().port}`;
});

after(() => new Promise((resolve) => server.close(resolve)));

test("sends auth and parses health", async () => {
  seen = [];
  const api = new DoOnceClient({ baseUrl, apiKey: "dsk_test" });
  assert.deepEqual(await api.health(), { status: "ok" });
  assert.equal(seen[0].apiKey, "dsk_test");
});

test("creates and runs a workflow", async () => {
  const api = new DoOnceClient({ baseUrl });
  const definition = { schemaVersion: "2.0", name: "Demo", blocks: [] };
  assert.equal((await api.createWorkflow(definition, "test")).workflow.slug, "demo");
  assert.equal((await api.runWorkflow("demo", { x: 1 })).run.id, "r1");
  assert.deepEqual(seen.at(-1).body, { inputs: { x: 1 } });
});

test("downloads artifact bytes", async () => {
  const api = new DoOnceClient({ baseUrl });
  const bytes = await api.downloadArtifact("a1");
  assert.deepEqual(Array.from(bytes), [0x89, 0x50, 0x4e, 0x47]);
});

test("maps problem details to DoOnceError", async () => {
  const api = new DoOnceClient({ baseUrl });
  await assert.rejects(api.getRun("missing"), (error) => {
    assert.ok(error instanceof DoOnceError);
    assert.equal(error.status, 404);
    assert.equal(error.title, "Not found");
    assert.equal(error.detail, "missing");
    return true;
  });
});
