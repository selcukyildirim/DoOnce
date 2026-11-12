"""Small dependency-free client for the DoOnce REST API.

The client targets the control-plane API (`/api/v1`). It uses only the Python
standard library so it can be vendored into restricted enterprise environments.

Example::

    from doonce import DoOnce

    api = DoOnce(api_key="dsk_...")
    slug = api.create_workflow(definition)["workflow"]["slug"]
    run = api.run_workflow(slug)["run"]
    result = api.wait_for_run(run["id"])
    artifacts = api.list_run_artifacts(run["id"])["artifacts"]
"""

from __future__ import annotations

import json
import os
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any, Mapping, Optional, Tuple, Union

JSON = Any


class DoOnceError(Exception):
    """Raised when the DoOnce API returns an error or is unreachable."""

    def __init__(
        self,
        status: int,
        title: str,
        detail: Optional[str] = None,
        body: Optional[JSON] = None,
    ) -> None:
        self.status = status
        self.title = title
        self.detail = detail
        self.body = body
        message = f"{status} {title}" if status else title
        if detail:
            message = f"{message}: {detail}"
        super().__init__(message)


class DoOnce:
    """Synchronous DoOnce API client.

    Args:
        base_url: Control-plane URL. May omit `/api/v1`.
        api_key: API key. Defaults to ``DOONCE_API_KEY``.
        timeout: Request timeout in seconds.
    """

    def __init__(
        self,
        base_url: Optional[str] = None,
        api_key: Optional[str] = None,
        timeout: float = 60.0,
    ) -> None:
        base = base_url or os.environ.get("DOONCE_API_URL") or "http://127.0.0.1:8080"
        base = base.rstrip("/")
        if not base.endswith("/api/v1"):
            base = f"{base}/api/v1"
        self.base_url = base
        self.api_key = api_key or os.environ.get("DOONCE_API_KEY")
        self.timeout = timeout

    def __repr__(self) -> str:
        return f"DoOnce(base_url={self.base_url!r})"

    def _url(self, path: str, params: Optional[Mapping[str, Any]] = None) -> str:
        url = f"{self.base_url}/{path.lstrip('/')}"
        if params:
            query = urllib.parse.urlencode(
                {key: value for key, value in params.items() if value is not None}
            )
            if query:
                url = f"{url}?{query}"
        return url

    def _request(
        self,
        method: str,
        path: str,
        *,
        params: Optional[Mapping[str, Any]] = None,
        body: Optional[JSON] = None,
    ) -> JSON:
        data = None if body is None else json.dumps(body).encode("utf-8")
        request = urllib.request.Request(self._url(path, params), data=data, method=method)
        request.add_header("accept", "application/json")
        if data is not None:
            request.add_header("content-type", "application/json")
        if self.api_key:
            request.add_header("x-api-key", self.api_key)
        try:
            with urllib.request.urlopen(request, timeout=self.timeout) as response:
                raw = response.read()
                if not raw:
                    return None
                return json.loads(raw)
        except urllib.error.HTTPError as error:
            raw = error.read()
            body_value: JSON = None
            title = error.reason or "HTTP error"
            detail = None
            try:
                body_value = json.loads(raw) if raw else None
                if isinstance(body_value, dict):
                    title = str(body_value.get("title") or title)
                    detail = body_value.get("detail")
            except (ValueError, TypeError):
                detail = raw.decode("utf-8", errors="replace")[:2000] if raw else None
            raise DoOnceError(error.code, title, detail, body_value) from error
        except urllib.error.URLError as error:
            raise DoOnceError(0, "Connection error", str(error.reason)) from error

    def _download(self, path: str) -> Tuple[bytes, Mapping[str, str]]:
        request = urllib.request.Request(self._url(path), method="GET")
        if self.api_key:
            request.add_header("x-api-key", self.api_key)
        try:
            with urllib.request.urlopen(request, timeout=self.timeout) as response:
                return response.read(), dict(response.headers.items())
        except urllib.error.HTTPError as error:
            raw = error.read()
            detail = raw.decode("utf-8", errors="replace")[:2000] if raw else None
            raise DoOnceError(error.code, error.reason or "HTTP error", detail) from error
        except urllib.error.URLError as error:
            raise DoOnceError(0, "Connection error", str(error.reason)) from error

    def health(self) -> JSON:
        return self._request("GET", "/health")

    def list_workflows(self, limit: int = 50) -> JSON:
        return self._request("GET", "/workflows", params={"limit": limit})

    def get_workflow(self, reference: str) -> JSON:
        return self._request("GET", f"/workflows/{_quote(reference)}")

    def create_workflow(self, definition: Mapping[str, Any], reason: Optional[str] = None) -> JSON:
        body: dict[str, Any] = {"definition": definition}
        if reason:
            body["reason"] = reason
        return self._request("POST", "/workflows", body=body)

    def update_workflow(
        self, reference: str, definition: Mapping[str, Any], reason: Optional[str] = None
    ) -> JSON:
        body: dict[str, Any] = {"definition": definition}
        if reason:
            body["reason"] = reason
        return self._request("PUT", f"/workflows/{_quote(reference)}", body=body)

    def delete_workflow(self, reference: str) -> JSON:
        return self._request("DELETE", f"/workflows/{_quote(reference)}")

    def validate_workflow(self, definition: Mapping[str, Any]) -> JSON:
        return self._request("POST", "/workflows/validate", body=definition)

    def run_workflow(self, reference: str, inputs: Optional[Mapping[str, Any]] = None) -> JSON:
        return self._request(
            "POST",
            f"/workflows/{_quote(reference)}/runs",
            body={"inputs": dict(inputs or {})},
        )

    def list_runs(
        self, workflow_id: Optional[str] = None, limit: int = 50
    ) -> JSON:
        return self._request(
            "GET",
            "/runs",
            params={"workflow_id": workflow_id, "limit": limit},
        )

    def get_run(self, run_id: str) -> JSON:
        return self._request("GET", f"/runs/{_quote(run_id)}")

    def cancel_run(self, run_id: str) -> JSON:
        return self._request("POST", f"/runs/{_quote(run_id)}/cancel")

    def wait_for_run(
        self,
        run_id: str,
        timeout: float = 300.0,
        interval: float = 2.0,
    ) -> JSON:
        deadline = time.monotonic() + timeout
        while True:
            run = self.get_run(run_id)["run"]
            if run["status"] in {"completed", "failed", "cancelled", "awaiting_approval"}:
                return run
            if time.monotonic() >= deadline:
                raise TimeoutError(f"run {run_id} did not finish within {timeout}s")
            time.sleep(interval)

    def list_run_artifacts(self, run_id: str) -> JSON:
        return self._request("GET", f"/runs/{_quote(run_id)}/artifacts")

    def list_task_artifacts(self, task_id: str) -> JSON:
        return self._request("GET", f"/tasks/{_quote(task_id)}/artifacts")

    def download_artifact(
        self, artifact_id: str, output: Optional[Union[str, Path]] = None
    ) -> bytes:
        data, _headers = self._download(f"/artifacts/{_quote(artifact_id)}")
        if output is not None:
            Path(output).write_bytes(data)
        return data

    def list_tasks(self, limit: int = 50) -> JSON:
        return self._request("GET", "/tasks", params={"limit": limit})

    def create_task(
        self,
        prompt: str,
        *,
        url: Optional[str] = None,
        navigation_goal: Optional[str] = None,
        data_schema: Optional[Mapping[str, Any]] = None,
        error_codes: Optional[list[str]] = None,
        model: Optional[str] = None,
        max_steps: Optional[int] = None,
    ) -> JSON:
        body: dict[str, Any] = {"prompt": prompt}
        if url is not None:
            body["url"] = url
        if navigation_goal is not None:
            body["navigationGoal"] = navigation_goal
        if data_schema is not None:
            body["dataSchema"] = data_schema
        if error_codes is not None:
            body["errorCodes"] = error_codes
        if model is not None:
            body["model"] = model
        if max_steps is not None:
            body["maxSteps"] = max_steps
        return self._request("POST", "/tasks", body=body)

    def get_task(self, task_id: str) -> JSON:
        return self._request("GET", f"/tasks/{_quote(task_id)}")

    def cancel_task(self, task_id: str) -> JSON:
        return self._request("POST", f"/tasks/{_quote(task_id)}/cancel")

    def list_approvals(
        self,
        status: Optional[str] = None,
        run_id: Optional[str] = None,
        task_id: Optional[str] = None,
        limit: int = 50,
    ) -> JSON:
        return self._request(
            "GET",
            "/approvals",
            params={
                "status": status,
                "run_id": run_id,
                "task_id": task_id,
                "limit": limit,
            },
        )

    def resolve_approval(self, approval_id: str, decision: str) -> JSON:
        return self._request(
            "POST",
            f"/approvals/{_quote(approval_id)}",
            body={"decision": decision},
        )

    def metrics(self) -> JSON:
        return self._request("GET", "/metrics")


def _quote(value: str) -> str:
    return urllib.parse.quote(value, safe="")
