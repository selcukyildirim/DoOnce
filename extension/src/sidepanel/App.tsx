import { useCallback, useEffect, useState } from "react";
import type { ExecutionState, Workflow, WorkflowSummary } from "../shared/types";
import { bg, onBackground } from "./api";

type View = "home" | "record" | "run" | "save";

export function App() {
  const [connected, setConnected] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [workflows, setWorkflows] = useState<WorkflowSummary[]>([]);
  const [view, setView] = useState<View>("home");
  const [recording, setRecording] = useState(false);
  const [draft, setDraft] = useState<{ workflow: Workflow; warnings: string[] } | null>(null);
  const [saveName, setSaveName] = useState("");
  const [execution, setExecution] = useState<ExecutionState | null>(null);
  const [hostInfo, setHostInfo] = useState<string>("");

  const refreshList = useCallback(async () => {
    const r = await bg({ type: "host", method: "listWorkflows" });
    setWorkflows(r.workflows ?? []);
  }, []);

  const connect = useCallback(async () => {
    setError(null);
    try {
      const r = await bg({ type: "connect" });
      setConnected(true);
      setHostInfo(`DoOnce ${r.version} · ${r.dataDir}`);
      await refreshList();
    } catch (e) {
      setConnected(false);
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [refreshList]);

  useEffect(() => {
    connect();
    return onBackground((msg) => {
      if (msg.type === "execution") setExecution(msg.execution as ExecutionState | null);
    });
  }, [connect]);

  async function startRecord() {
    setError(null);
    await bg({ type: "startRecording" });
    setRecording(true);
    setView("record");
  }

  async function stopRecord() {
    const r = await bg({ type: "stopRecording" });
    setRecording(false);
    const compiled = await bg({
      type: "host",
      method: "compileRecording",
      params: { events: r.events ?? [] },
    });
    const wf = compiled.compiled!.workflow;
    setDraft({ workflow: wf, warnings: compiled.compiled!.warnings });
    setSaveName(wf.name);
    setView("save");
  }

  async function saveDraft() {
    if (!draft) return;
    const wf = { ...draft.workflow, name: saveName.trim() || draft.workflow.name };
    await bg({ type: "host", method: "saveWorkflow", params: { workflow: wf, reason: "Recorded in browser" } });
    setDraft(null);
    setView("home");
    await refreshList();
  }

  async function runWorkflow(slug: string) {
    setError(null);
    setView("run");
    setExecution(null);
    const r = await bg({ type: "host", method: "run", params: { ref: slug } });
    const run = r.run!;
    setExecution({
      executionId: run.executionId,
      workflowSlug: run.slug,
      workflowName: run.name,
      status: "running",
      steps: run.steps.map((s: { id: string; summary: string }) => ({
        id: s.id,
        summary: s.summary,
        status: "pending" as const,
      })),
    });
  }

  return (
    <div className="app">
      <h1>DoOnce</h1>
      <p className="sub">{connected ? hostInfo : "Not connected to the DoOnce engine"}</p>

      {error && <div className="banner error">{error}</div>}

      {!connected && (
        <div className="card">
          <p className="meta">Install the CLI, build `doonce`, then run:</p>
          <code>doonce setup</code>
          <div className="row" style={{ marginTop: 12 }}>
            <button type="button" onClick={() => connect()}>
              Retry connection
            </button>
          </div>
        </div>
      )}

      {connected && view === "home" && (
        <>
          <div className="card row">
            <button type="button" onClick={startRecord} disabled={recording}>
              Record
            </button>
            <button type="button" className="secondary" onClick={() => refreshList()}>
              Refresh
            </button>
          </div>
          <div className="card">
            <strong>My automations</strong>
            {workflows.length === 0 && <p className="meta">Nothing saved yet.</p>}
            {workflows.map((w) => (
              <div className="list-item" key={w.id}>
                <div>
                  <div>{w.name}</div>
                  <div className="meta">
                    {w.slug} · {w.stepCount} steps · {w.lastStatus ?? "never run"}
                  </div>
                </div>
                <button type="button" className="secondary" onClick={() => runWorkflow(w.slug)}>
                  Run
                </button>
              </div>
            ))}
          </div>
        </>
      )}

      {view === "record" && (
        <div className="card">
          <strong>Recording…</strong>
          <p className="meta">Use the page normally. Sensitive fields are masked.</p>
          <button type="button" className="danger" onClick={stopRecord}>
            Stop
          </button>
        </div>
      )}

      {view === "save" && draft && (
        <div className="card">
          <strong>Save automation</strong>
          <label className="field">
            Name
            <input type="text" value={saveName} onChange={(e) => setSaveName(e.target.value)} />
          </label>
          <p className="meta">{draft.workflow.steps.length} steps</p>
          {draft.warnings.map((w) => (
            <p className="meta" key={w}>
              {w}
            </p>
          ))}
          <div className="row">
            <button type="button" onClick={saveDraft}>
              Save
            </button>
            <button type="button" className="secondary" onClick={() => setView("home")}>
              Discard
            </button>
          </div>
        </div>
      )}

      {view === "run" && execution && (
        <div className="card">
          <strong>{execution.workflowName}</strong>
          <p className="meta">Status: {execution.status}</p>
          {execution.error && <div className="banner error">{execution.error}</div>}
          {execution.approval && (
            <div className="banner ok">
              <div>{execution.approval.summary}</div>
              <div className="meta">{execution.approval.reason}</div>
              <div className="row" style={{ marginTop: 8 }}>
                <button
                  type="button"
                  onClick={() => bg({ type: "approve", approved: true }).then(() => setExecution({ ...execution, approval: null }))}
                >
                  Approve
                </button>
                <button
                  type="button"
                  className="secondary"
                  onClick={() => bg({ type: "approve", approved: false })}
                >
                  Deny
                </button>
              </div>
            </div>
          )}
          <div className="steps">
            {execution.steps.map((s) => (
              <div key={s.id} className={`step ${s.status}`}>
                {s.summary} — {s.status}
              </div>
            ))}
          </div>
          <div className="row" style={{ marginTop: 12 }}>
            <button type="button" className="secondary" onClick={() => bg({ type: "cancelRun" })}>
              Stop
            </button>
            <button type="button" className="secondary" onClick={() => setView("home")}>
              Back
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
