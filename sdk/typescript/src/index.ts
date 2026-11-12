export type JsonObject = Record<string, unknown>;

export interface DoOnceOptions {
  baseUrl?: string;
  apiKey?: string;
  timeoutMs?: number;
  fetch?: typeof fetch;
}

export interface WorkflowDefinition extends JsonObject {
  schemaVersion: string;
  name: string;
}

export interface Workflow {
  id: string;
  tenantId: string;
  slug: string;
  name: string;
  currentVersion: number;
  createdAt: string;
  updatedAt: string;
  [key: string]: unknown;
}

export interface WorkflowRun {
  id: string;
  tenantId: string;
  workflowId: string;
  status: string;
  trigger: string;
  inputs?: JsonObject;
  outputs?: JsonObject;
  error?: string | null;
  createdAt: string;
  [key: string]: unknown;
}

export interface Task {
  id: string;
  tenantId: string;
  prompt: string;
  status: string;
  url?: string | null;
  output?: unknown;
  error?: string | null;
  createdAt: string;
  [key: string]: unknown;
}

export interface Artifact {
  id: string;
  kind: string;
  filename: string;
  contentType: string;
  size: number;
  createdAt: string;
  [key: string]: unknown;
}

export interface Metrics {
  workflows: number;
  tasks: number;
  runs: number;
  runsCompleted: number;
  runsFailed: number;
  runsQueued: number;
  tasksQueued: number;
  pendingApprovals: number;
  artifacts: number;
  successRate: number;
  avgRunDurationMs?: number | null;
  [key: string]: unknown;
}

export interface CreateTaskOptions {
  url?: string;
  navigationGoal?: string;
  dataSchema?: JsonObject;
  errorCodes?: string[];
  model?: string;
  maxSteps?: number;
}

export interface WaitForRunOptions {
  timeoutMs?: number;
  intervalMs?: number;
}

export class DoOnceError extends Error {
  readonly status: number;
  readonly title: string;
  readonly detail?: string;
  readonly body?: unknown;

  constructor(status: number, title: string, detail?: string, body?: unknown) {
    super(detail ? `${status} ${title}: ${detail}` : `${status} ${title}`);
    this.name = "DoOnceError";
    this.status = status;
    this.title = title;
    this.detail = detail;
    this.body = body;
  }
}

export class DoOnceClient {
  readonly baseUrl: string;
  readonly apiKey?: string;
  private readonly timeoutMs: number;
  private readonly fetchImpl: typeof fetch;

  constructor(options: DoOnceOptions = {}) {
    const env = (
      globalThis as {
        process?: { env?: Record<string, string | undefined> };
      }
    ).process?.env;
    const base = options.baseUrl ?? env?.DOONCE_API_URL ?? "http://127.0.0.1:8080";
    const trimmed = base.replace(/\/+$/, "");
    this.baseUrl = trimmed.endsWith("/api/v1") ? trimmed : `${trimmed}/api/v1`;
    this.apiKey = options.apiKey ?? env?.DOONCE_API_KEY;
    this.timeoutMs = options.timeoutMs ?? 60_000;
    this.fetchImpl = options.fetch ?? globalThis.fetch.bind(globalThis);
  }

  private url(path: string, params?: Record<string, string | number | undefined>): string {
    const url = new URL(`${this.baseUrl}/${path.replace(/^\/+/, "")}`);
    for (const [key, value] of Object.entries(params ?? {})) {
      if (value !== undefined) {
        url.searchParams.set(key, String(value));
      }
    }
    return url.toString();
  }

  private headers(json: boolean): Record<string, string> {
    const headers: Record<string, string> = { accept: "application/json" };
    if (json) {
      headers["content-type"] = "application/json";
    }
    if (this.apiKey) {
      headers["x-api-key"] = this.apiKey;
    }
    return headers;
  }

  private async request<T>(
    method: string,
    path: string,
    options: {
      params?: Record<string, string | number | undefined>;
      body?: unknown;
    } = {},
  ): Promise<T> {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), this.timeoutMs);
    try {
      const response = await this.fetchImpl(this.url(path, options.params), {
        method,
        headers: this.headers(options.body !== undefined),
        body: options.body === undefined ? undefined : JSON.stringify(options.body),
        signal: controller.signal,
      });
      const text = await response.text();
      let parsed: unknown = undefined;
      if (text) {
        try {
          parsed = JSON.parse(text);
        } catch {
          parsed = text;
        }
      }
      if (!response.ok) {
        const problem = isObject(parsed) ? parsed : undefined;
        throw new DoOnceError(
          response.status,
          typeof problem?.title === "string" ? problem.title : response.statusText,
          typeof problem?.detail === "string" ? problem.detail : undefined,
          parsed,
        );
      }
      return parsed as T;
    } catch (error) {
      if (error instanceof DoOnceError) {
        throw error;
      }
      if (error instanceof Error && error.name === "AbortError") {
        throw new DoOnceError(0, "Request timed out", `after ${this.timeoutMs}ms`);
      }
      throw new DoOnceError(0, "Network error", error instanceof Error ? error.message : String(error));
    } finally {
      clearTimeout(timer);
    }
  }

  private async download(path: string): Promise<Uint8Array> {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), this.timeoutMs);
    try {
      const response = await this.fetchImpl(this.url(path), {
        method: "GET",
        headers: this.headers(false),
        signal: controller.signal,
      });
      if (!response.ok) {
        const text = await response.text();
        throw new DoOnceError(response.status, response.statusText, text.slice(0, 2000));
      }
      return new Uint8Array(await response.arrayBuffer());
    } catch (error) {
      if (error instanceof DoOnceError) {
        throw error;
      }
      throw new DoOnceError(0, "Network error", error instanceof Error ? error.message : String(error));
    } finally {
      clearTimeout(timer);
    }
  }

  health(): Promise<unknown> {
    return this.request("GET", "/health");
  }

  listWorkflows(limit = 50): Promise<{ workflows: Workflow[] }> {
    return this.request("GET", "/workflows", { params: { limit } });
  }

  getWorkflow(reference: string): Promise<{ workflow: Workflow; definition: WorkflowDefinition }> {
    return this.request("GET", `/workflows/${encodeURIComponent(reference)}`);
  }

  createWorkflow(
    definition: WorkflowDefinition,
    reason?: string,
  ): Promise<{ workflow: Workflow }> {
    return this.request("POST", "/workflows", {
      body: reason ? { definition, reason } : { definition },
    });
  }

  updateWorkflow(
    reference: string,
    definition: WorkflowDefinition,
    reason?: string,
  ): Promise<{ workflow: Workflow; version: unknown }> {
    return this.request("PUT", `/workflows/${encodeURIComponent(reference)}`, {
      body: reason ? { definition, reason } : { definition },
    });
  }

  deleteWorkflow(reference: string): Promise<unknown> {
    return this.request("DELETE", `/workflows/${encodeURIComponent(reference)}`);
  }

  validateWorkflow(definition: WorkflowDefinition): Promise<unknown> {
    return this.request("POST", "/workflows/validate", { body: definition });
  }

  runWorkflow(reference: string, inputs: JsonObject = {}): Promise<{ run: WorkflowRun }> {
    return this.request("POST", `/workflows/${encodeURIComponent(reference)}/runs`, {
      body: { inputs },
    });
  }

  listRuns(workflowId?: string, limit = 50): Promise<{ runs: WorkflowRun[] }> {
    return this.request("GET", "/runs", { params: { workflow_id: workflowId, limit } });
  }

  getRun(runId: string): Promise<{ run: WorkflowRun; blocks: unknown[] }> {
    return this.request("GET", `/runs/${encodeURIComponent(runId)}`);
  }

  cancelRun(runId: string): Promise<{ run: WorkflowRun }> {
    return this.request("POST", `/runs/${encodeURIComponent(runId)}/cancel`);
  }

  async waitForRun(runId: string, options: WaitForRunOptions = {}): Promise<WorkflowRun> {
    const timeoutMs = options.timeoutMs ?? 300_000;
    const intervalMs = options.intervalMs ?? 2_000;
    const deadline = Date.now() + timeoutMs;
    for (;;) {
      const { run } = await this.getRun(runId);
      if (["completed", "failed", "cancelled", "awaiting_approval"].includes(run.status)) {
        return run;
      }
      if (Date.now() >= deadline) {
        throw new DoOnceError(0, "Run timed out", `run ${runId} did not finish within ${timeoutMs}ms`);
      }
      await new Promise((resolve) => setTimeout(resolve, intervalMs));
    }
  }

  listRunArtifacts(runId: string): Promise<{ artifacts: Artifact[] }> {
    return this.request("GET", `/runs/${encodeURIComponent(runId)}/artifacts`);
  }

  listTaskArtifacts(taskId: string): Promise<{ artifacts: Artifact[] }> {
    return this.request("GET", `/tasks/${encodeURIComponent(taskId)}/artifacts`);
  }

  downloadArtifact(artifactId: string): Promise<Uint8Array> {
    return this.download(`/artifacts/${encodeURIComponent(artifactId)}`);
  }

  listTasks(limit = 50): Promise<{ tasks: Task[] }> {
    return this.request("GET", "/tasks", { params: { limit } });
  }

  createTask(prompt: string, options: CreateTaskOptions = {}): Promise<{ task: Task }> {
    const body: JsonObject = { prompt };
    if (options.url !== undefined) body.url = options.url;
    if (options.navigationGoal !== undefined) body.navigationGoal = options.navigationGoal;
    if (options.dataSchema !== undefined) body.dataSchema = options.dataSchema;
    if (options.errorCodes !== undefined) body.errorCodes = options.errorCodes;
    if (options.model !== undefined) body.model = options.model;
    if (options.maxSteps !== undefined) body.maxSteps = options.maxSteps;
    return this.request("POST", "/tasks", { body });
  }

  getTask(taskId: string): Promise<{ task: Task; blocks?: unknown[] }> {
    return this.request("GET", `/tasks/${encodeURIComponent(taskId)}`);
  }

  cancelTask(taskId: string): Promise<{ task: Task }> {
    return this.request("POST", `/tasks/${encodeURIComponent(taskId)}/cancel`);
  }

  listApprovals(
    options: { status?: string; runId?: string; taskId?: string; limit?: number } = {},
  ): Promise<{ approvals: unknown[] }> {
    return this.request("GET", "/approvals", {
      params: {
        status: options.status,
        run_id: options.runId,
        task_id: options.taskId,
        limit: options.limit ?? 50,
      },
    });
  }

  resolveApproval(approvalId: string, decision: "approve" | "deny"): Promise<unknown> {
    return this.request("POST", `/approvals/${encodeURIComponent(approvalId)}`, {
      body: { decision },
    });
  }

  metrics(): Promise<{ metrics: Metrics }> {
    return this.request("GET", "/metrics");
  }
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}
