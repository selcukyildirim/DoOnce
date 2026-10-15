// Mirrors the Rust types in crates/doonce-core. Keep field names in sync.

export interface Target {
  role?: string;
  name?: string;
  tag?: string;
  text?: string;
  label?: string;
  attributes?: Record<string, string>;
  selectors?: string[];
  context?: string;
}

export interface Condition {
  urlContains?: string;
  textContains?: string;
  visible?: Target;
  hidden?: Target;
}

export type RecordedKind =
  | { kind: "start"; title?: string }
  | { kind: "navigate"; transition?: string }
  | { kind: "click"; target: Target }
  | { kind: "fill"; target: Target; value: string | null; sensitive: boolean }
  | { kind: "select"; target: Target; value: string }
  | { kind: "check"; target: Target; checked: boolean }
  | { kind: "press"; target?: Target; key: string }
  | { kind: "download"; filename?: string };

export type RecordedEvent = RecordedKind & { ts: number; url?: string; tabId?: number };

export type StepStatus =
  | "pending"
  | "running"
  | "completed"
  | "failed"
  | "awaiting_approval"
  | "cancelled"
  | "skipped";

export interface Parameter {
  name: string;
  type?: "string" | "date" | "number" | "secret";
  default?: string;
  description?: string;
}

export interface Step {
  id: string;
  type: string;
  description?: string;
  [key: string]: unknown;
}

export interface Workflow {
  schemaVersion: string;
  name: string;
  description?: string;
  parameters?: Parameter[];
  steps: Step[];
}

export interface WorkflowSummary {
  id: number;
  slug: string;
  name: string;
  version: number;
  stepCount: number;
  parameterCount: number;
  updatedAt: string;
  lastRunAt?: string | null;
  lastStatus?: string | null;
}

export interface ExecutionStep {
  id: string;
  summary: string;
  status: StepStatus;
  error?: string | null;
  durationMs?: number;
}

export interface ExecutionState {
  executionId: number;
  workflowSlug: string;
  workflowName: string;
  status: "running" | "completed" | "failed" | "cancelled";
  steps: ExecutionStep[];
  error?: string | null;
  approval?: { stepId: string; summary: string; reason: string } | null;
}
