const BASE = "/api/v1";

let apiKey = localStorage.getItem("doonce.apiKey") ?? "";

export function setApiKey(key: string) {
  apiKey = key.trim();
  localStorage.setItem("doonce.apiKey", apiKey);
}

export function clearApiKey() {
  apiKey = "";
  localStorage.removeItem("doonce.apiKey");
}

export function hasApiKey() {
  return apiKey.length > 0;
}

function cookie(name: string): string | undefined {
  const prefix = `${name}=`;
  return document.cookie
    .split(";")
    .map((part) => part.trim())
    .find((part) => part.startsWith(prefix))
    ?.slice(prefix.length);
}

export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
    this.name = "ApiError";
  }
}

export async function api<T>(path: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers);
  if (init.body) headers.set("content-type", "application/json");
  if (apiKey) headers.set("x-api-key", apiKey);
  const method = (init.method ?? "GET").toUpperCase();
  if (!["GET", "HEAD", "OPTIONS"].includes(method)) {
    const csrf = cookie("doonce_csrf");
    if (csrf) headers.set("x-csrf-token", csrf);
  }
  const response = await fetch(BASE + path, { ...init, headers, credentials: "same-origin" });
  if (!response.ok) {
    let message = `${response.status} ${response.statusText}`;
    try {
      const problem = (await response.json()) as { detail?: string; title?: string };
      message = problem.detail || problem.title || message;
    } catch {
      /* non-JSON error */
    }
    throw new ApiError(response.status, message);
  }
  if (response.status === 204) {
    return undefined as T;
  }
  return (await response.json()) as T;
}

export async function apiBlob(path: string): Promise<string> {
  const headers = new Headers();
  if (apiKey) headers.set("x-api-key", apiKey);
  const response = await fetch(BASE + path, { headers, credentials: "same-origin" });
  if (!response.ok) {
    throw new Error(`${response.status} ${response.statusText}`);
  }
  return URL.createObjectURL(await response.blob());
}

export const sampleWorkflow = `{
  "schemaVersion": "2.0",
  "name": "Google search digest",
  "description": "Search Google and return the top results as structured data.",
  "blocks": [
    {
      "id": "search",
      "type": "browserTask",
      "prompt": "Go to Google, search for \\"best rust web framework\\" and return the top 3 organic results with title, url and snippet.",
      "url": "https://www.google.com",
      "dataSchema": {
        "type": "object",
        "properties": {
          "results": {
            "type": "array",
            "items": {
              "type": "object",
              "properties": {
                "title": { "type": "string" },
                "url": { "type": "string" },
                "snippet": { "type": "string" }
              }
            }
          }
        }
      },
      "maxSteps": 15,
      "next": "done"
    },
    {
      "id": "done",
      "type": "terminate",
      "status": "completed"
    }
  ]
}`;

export const sampleTaskSchema = `{
  "type": "object",
  "properties": {
    "results": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "title": { "type": "string" },
          "url": { "type": "string" }
        }
      }
    }
  },
  "required": ["results"]
}`;

export function pretty(value: unknown): string {
  return JSON.stringify(value, null, 2);
}

export function parseInput(text: string, label: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    throw new Error(`${label} is not valid JSON`);
  }
}
