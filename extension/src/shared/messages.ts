/** Messages between the side panel and the service worker. */

import type { ExecutionState, RecordedEvent, Workflow, WorkflowSummary } from "./types";

export type HostEvent =
  | { event: "progress"; executionId: number; kind: string; [key: string]: unknown }
  | { event: "approval"; executionId: number; stepId: string; summary: string; reason: string }
  | { event: "finished"; executionId: number; outcome: { status: string; failedStep?: string; failure?: { message: string } } };

export type BgRequest =
  | { type: "connect" }
  | { type: "host"; method: string; params?: Record<string, unknown> }
  | { type: "startRecording" }
  | { type: "stopRecording" }
  | { type: "getRecording" }
  | { type: "approve"; approved: boolean }
  | { type: "cancelRun" };

export type BgOk = {
  ok: true;
  connected?: boolean;
  version?: string;
  dataDir?: string;
  workflows?: WorkflowSummary[];
  workflow?: unknown;
  compiled?: { workflow: Workflow; warnings: string[]; steps: unknown[] };
  saved?: unknown;
  run?: { executionId: number; steps: { id: string; summary: string }[]; name: string; slug: string };
  events?: RecordedEvent[];
  recording?: boolean;
  execution?: ExecutionState | null;
};

export type BgResponse = BgOk | { ok: false; error: string };

export const HOST_NAME = "com.doonce.host";
