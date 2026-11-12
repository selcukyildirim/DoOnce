import json
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from doonce import DoOnce, DoOnceError


class Handler(BaseHTTPRequestHandler):
    seen = []

    def log_message(self, *_args):
        pass

    def _send(self, status, body, content_type="application/json"):
        payload = body if isinstance(body, bytes) else json.dumps(body).encode()
        self.send_response(status)
        self.send_header("content-type", content_type)
        self.send_header("content-length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def do_GET(self):
        Handler.seen.append(("GET", self.path, self.headers.get("x-api-key")))
        if self.path == "/api/v1/health":
            self._send(200, {"status": "ok"})
        elif self.path.startswith("/api/v1/workflows"):
            self._send(200, {"workflows": [{"slug": "demo"}]})
        elif self.path == "/api/v1/artifacts/a1":
            self._send(200, b"\x89PNG\r\n", "image/png")
        else:
            self._send(404, {"title": "Not found", "detail": "missing"})

    def do_POST(self):
        length = int(self.headers.get("content-length", "0"))
        body = json.loads(self.rfile.read(length) or b"{}")
        Handler.seen.append(("POST", self.path, self.headers.get("x-api-key"), body))
        if self.path == "/api/v1/workflows":
            self._send(201, {"workflow": {"slug": "demo"}})
        elif self.path == "/api/v1/workflows/demo/runs":
            self._send(202, {"run": {"id": "r1", "status": "queued"}})
        else:
            self._send(404, {"title": "Not found"})


class ClientTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()
        cls.base = f"http://127.0.0.1:{cls.server.server_port}"

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()

    def setUp(self):
        Handler.seen.clear()
        self.api = DoOnce(base_url=self.base, api_key="dsk_test")

    def test_health_and_auth_header(self):
        self.assertEqual(self.api.health(), {"status": "ok"})
        self.assertEqual(Handler.seen[0][2], "dsk_test")

    def test_create_workflow_and_run(self):
        definition = {"schemaVersion": "2.0", "name": "Demo", "blocks": []}
        created = self.api.create_workflow(definition, reason="test")
        self.assertEqual(created["workflow"]["slug"], "demo")
        run = self.api.run_workflow("demo", {"x": 1})
        self.assertEqual(run["run"]["id"], "r1")
        self.assertEqual(Handler.seen[-1][3], {"inputs": {"x": 1}})

    def test_download_artifact(self):
        data = self.api.download_artifact("a1")
        self.assertTrue(data.startswith(b"\x89PNG"))

    def test_problem_details_become_exception(self):
        with self.assertRaises(DoOnceError) as caught:
            self.api.get_run("missing")
        self.assertEqual(caught.exception.status, 404)
        self.assertEqual(caught.exception.title, "Not found")
        self.assertEqual(caught.exception.detail, "missing")


if __name__ == "__main__":
    unittest.main()
