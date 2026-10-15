import type { BgRequest, BgResponse, HostEvent } from "../shared/messages";
import type { ExecutionState, StepStatus } from "../shared/types";
import * as host from "./nativeHost";
import { handleDriver } from "./driver";
import * as recording from "./recording";

host.setDriverHandler(handleDriver);

// Side panel opens when the toolbar icon is clicked (do not also use action.onClicked).
chrome.sidePanel.setPanelBehavior({ openPanelOnActionClick: true }).catch(() => {});

let execution: ExecutionState | null = null;

host.onHostEvent((raw) => {
  const ev = raw as HostEvent;
  if (ev.event === "progress" && execution) {
    const body = raw as { stepId?: string; status?: string; error?: string; durationMs?: number };
    const stepId = String(body.stepId ?? "");
    const status = String(body.status ?? "") as StepStatus;
    const error = (raw as { error?: string }).error ?? null;
    const durationMs = Number((raw as { durationMs?: number }).durationMs ?? 0);
    execution.steps = execution.steps.map((s) =>
      s.id === stepId ? { ...s, status, error, durationMs } : s,
    );
    if (status === "running") execution.status = "running";
    broadcast({ type: "execution", execution: { ...execution } });
  }
  if (ev.event === "approval" && execution) {
    execution.approval = {
      stepId: ev.stepId,
      summary: ev.summary,
      reason: ev.reason,
    };
    broadcast({ type: "execution", execution: { ...execution } });
  }
  if (ev.event === "finished") {
    execution = execution
      ? {
          ...execution,
          status: ev.outcome.status as ExecutionState["status"],
          error: ev.outcome.failure?.message ?? null,
          approval: null,
        }
      : null;
    broadcast({ type: "execution", execution });
  }
  broadcast({ type: "hostEvent", event: raw });
});

function broadcast(msg: unknown) {
  chrome.runtime.sendMessage(msg).catch(() => {});
}

chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  if (msg?.channel === "recordEvent" && msg.payload) {
    recording.pushRecorded(msg.payload);
    sendResponse({ ok: true });
    return false;
  }
  if (msg?.type) {
    handle(msg as BgRequest)
      .then((r) => sendResponse(r))
      .catch((e: Error) => sendResponse({ ok: false, error: e.message } satisfies BgResponse));
    return true;
  }
  return false;
});

chrome.webNavigation.onCommitted.addListener((details) => {
  if (!recording.isRecording() || details.frameId !== 0) return;
  recording.pushNavigate(details.url, details.transitionType);
});

chrome.runtime.onInstalled.addListener(() => {
  chrome.sidePanel.setPanelBehavior({ openPanelOnActionClick: true }).catch(() => {});
});

async function handle(req: BgRequest): Promise<BgResponse> {
  switch (req.type) {
    case "connect": {
      const info = await host.connectHost();
      return { ok: true, connected: true, version: info.version, dataDir: info.dataDir };
    }
    case "host": {
      if (!host.hostConnected()) await host.connectHost();
      const result = await host.hostCall(req.method, req.params ?? {});
      if (req.method === "listWorkflows") return { ok: true, workflows: result as never };
      if (req.method === "compileRecording") return { ok: true, compiled: result as never };
      if (req.method === "saveWorkflow") return { ok: true, saved: result };
      if (req.method === "run") {
        const run = result as {
          executionId: number;
          steps: { id: string; summary: string }[];
          name: string;
          slug: string;
        };
        execution = {
          executionId: run.executionId,
          workflowSlug: run.slug,
          workflowName: run.name,
          status: "running",
          steps: run.steps.map((s) => ({ id: s.id, summary: s.summary, status: "pending" as StepStatus })),
          approval: null,
        };
        return { ok: true, run };
      }
      return { ok: true, workflow: result };
    }
    case "startRecording":
      await recording.startRecording();
      return { ok: true, recording: true };
    case "stopRecording": {
      recording.stopRecording();
      return { ok: true, events: recording.getEvents(), recording: false };
    }
    case "getRecording":
      return { ok: true, events: recording.getEvents(), recording: recording.isRecording() };
    case "approve":
      await host.hostCall("approve", { approved: req.approved });
      if (execution?.approval) execution.approval = null;
      return { ok: true, execution: execution ?? undefined };
    case "cancelRun":
      await host.hostCall("cancel", {});
      return { ok: true };
    default:
      return { ok: false, error: "unknown request" };
  }
}
