import { useEffect, useMemo, useState, type ReactNode } from "react";

interface Block {
  id: string;
  type: string;
  [key: string]: unknown;
}

interface WorkflowDoc {
  schemaVersion?: string;
  name?: string;
  description?: string;
  parameters?: unknown[];
  blocks?: Block[];
  [key: string]: unknown;
}

const PALETTE = [
  "browserAction",
  "browserTask",
  "extract",
  "validate",
  "forLoop",
  "conditional",
  "httpRequest",
  "code",
  "fileParse",
  "textPrompt",
  "sendEmail",
  "downloadFiles",
  "upload",
  "login",
  "humanApproval",
  "runWorkflow",
  "wait",
  "terminate",
  "goto",
];

function template(type: string, id: string): Block {
  switch (type) {
    case "browserAction":
      return { id, type, action: { kind: "navigate", url: "https://www.google.com/" } };
    case "browserTask":
      return { id, type, prompt: "Describe the browser task", maxSteps: 30 };
    case "extract":
      return { id, type, prompt: "Extract the requested data", schema: { type: "object" } };
    case "validate":
      return { id, type, prompt: "Validate the page state" };
    case "forLoop":
      return { id, type, over: "{{items}}", body: "", maxIterations: 100 };
    case "conditional":
      return { id, type, condition: { "==": [1, 1] }, then: "" };
    case "httpRequest":
      return { id, type, method: "GET", url: "https://www.googleapis.com/" };
    case "code":
      return { id, type, language: "javascript", code: "globalThis.result = 1;" };
    case "fileParse":
      return { id, type, artifact: "data.json", parser: "json" };
    case "textPrompt":
      return { id, type, prompt: "Write a short summary" };
    case "sendEmail":
      return { id, type, to: [], subject: "", body: "" };
    case "downloadFiles":
      return { id, type, prompt: "Download the requested files" };
    case "upload":
      return { id, type, artifact: "", destination: "" };
    case "login":
      return { id, type, credentialId: "", totp: false };
    case "humanApproval":
      return { id, type, message: "Approve this step" };
    case "runWorkflow":
      return { id, type, workflowId: "" };
    case "wait":
      return { id, type, condition: { textContains: "" } };
    case "terminate":
      return { id, type, status: "completed" };
    case "goto":
      return { id, type, target: "" };
    default:
      return { id, type };
  }
}

function uniqueId(blocks: Block[]): string {
  let index = blocks.length + 1;
  while (blocks.some((block) => block.id === `block-${index}`)) {
    index += 1;
  }
  return `block-${index}`;
}

function summary(block: Block): string {
  if (typeof block.label === "string" && block.label) return block.label;
  if (typeof block.prompt === "string" && block.prompt) return block.prompt;
  if (typeof block.description === "string" && block.description) return block.description;
  if (block.type === "httpRequest" && typeof block.url === "string") {
    return `${String(block.method ?? "GET")} ${block.url}`;
  }
  if (block.type === "browserAction" && typeof block.action === "object" && block.action) {
    const action = block.action as Record<string, unknown>;
    return `${String(action.kind ?? "action")} ${String(action.url ?? action.value ?? "")}`;
  }
  return block.type;
}

export function WorkflowBuilder({
  json,
  onChange,
}: {
  json: string;
  onChange: (value: string) => void;
}) {
  const doc = useMemo<WorkflowDoc | null>(() => {
    try {
      return JSON.parse(json) as WorkflowDoc;
    } catch {
      return null;
    }
  }, [json]);
  const blocks = useMemo(() => (Array.isArray(doc?.blocks) ? doc.blocks : []), [doc]);
  const [selected, setSelected] = useState<number | null>(null);
  const [blockDraft, setBlockDraft] = useState("");
  const [draftError, setDraftError] = useState<string | null>(null);

  useEffect(() => {
    if (selected != null && blocks[selected]) {
      setBlockDraft(JSON.stringify(blocks[selected], null, 2));
      setDraftError(null);
    }
  }, [selected, blocks]);

  if (!doc) {
    return <pre className="output">Fix the JSON in the editor before using the builder.</pre>;
  }

  function update(next: WorkflowDoc) {
    onChange(JSON.stringify(next, null, 2));
  }

  function updateBlock(index: number, block: Block) {
    update({ ...doc, blocks: blocks.map((item, position) => (position === index ? block : item)) });
  }

  function patchBlock(index: number, patch: Record<string, unknown>) {
    updateBlock(index, { ...blocks[index], ...patch });
  }

  function addBlock(type: string) {
    const block = template(type, uniqueId(blocks));
    update({ ...doc, blocks: [...blocks, block] });
    setSelected(blocks.length);
  }

  function removeBlock(index: number) {
    const next = blocks.filter((_, position) => position !== index);
    update({ ...doc, blocks: next });
    setSelected(next.length > 0 ? Math.min(index, next.length - 1) : null);
  }

  function moveBlock(index: number, direction: -1 | 1) {
    const target = index + direction;
    if (target < 0 || target >= blocks.length) return;
    const next = [...blocks];
    [next[index], next[target]] = [next[target], next[index]];
    update({ ...doc, blocks: next });
    setSelected(target);
  }

  function applyDraft() {
    if (selected == null) return;
    try {
      const block = JSON.parse(blockDraft) as Block;
      if (!block.id || !block.type) throw new Error("block needs id and type");
      updateBlock(selected, block);
      setDraftError(null);
    } catch (error) {
      setDraftError(error instanceof Error ? error.message : String(error));
    }
  }

  const current = selected != null ? blocks[selected] : null;
  const blockIds = blocks.map((block) => block.id);

  return (
    <div className="builder">
      <div className="builder-palette">
        <h4>Add block</h4>
        <div className="palette-grid">
          {PALETTE.map((type) => (
            <button key={type} className="ghost small" onClick={() => addBlock(type)}>
              {type}
            </button>
          ))}
        </div>
      </div>

      <div className="builder-list">
        <div className="card-head">
          <h4>Blocks</h4>
          <span className="muted small">{blocks.length} blocks</span>
        </div>
        {blocks.map((block, index) => (
          <div
            key={`${block.id}-${index}`}
            className={`builder-block ${selected === index ? "selected" : ""}`}
            onClick={() => setSelected(index)}
          >
            <div className="builder-block-main">
              <strong>{block.id}</strong>
              <span className="muted small">{block.type}</span>
              <span className="muted small clip">{summary(block)}</span>
            </div>
            <div className="row" onClick={(event) => event.stopPropagation()}>
              <button className="ghost small" onClick={() => moveBlock(index, -1)} title="Move up">
                ↑
              </button>
              <button className="ghost small" onClick={() => moveBlock(index, 1)} title="Move down">
                ↓
              </button>
              <button className="danger small" onClick={() => removeBlock(index)} title="Delete">
                ✕
              </button>
            </div>
          </div>
        ))}
        {blocks.length === 0 && <p className="muted">Add a block from the palette.</p>}
      </div>

      <div className="builder-inspector">
        {current && selected != null ? (
          <>
            <div className="card-head">
              <h4>{current.id}</h4>
              <span className="badge">{current.type}</span>
            </div>
            <Field label="ID">
              <input
                value={String(current.id ?? "")}
                onChange={(event) => patchBlock(selected, { id: event.target.value })}
              />
            </Field>
            <Field label="Label">
              <input
                value={String(current.label ?? "")}
                onChange={(event) => patchBlock(selected, { label: event.target.value || undefined })}
              />
            </Field>
            <Field label="Description">
              <textarea
                value={String(current.description ?? "")}
                onChange={(event) => patchBlock(selected, { description: event.target.value || undefined })}
              />
            </Field>
            <div className="grid2">
              <Field label="Next">
                <select
                  value={String(current.next ?? "")}
                  onChange={(event) => patchBlock(selected, { next: event.target.value || undefined })}
                >
                  <option value="">(fallthrough)</option>
                  {blockIds.map((id) => (
                    <option key={id} value={id}>
                      {id}
                    </option>
                  ))}
                </select>
              </Field>
              <Field label="On error">
                <select
                  value={String(current.onError ?? "")}
                  onChange={(event) => patchBlock(selected, { onError: event.target.value || undefined })}
                >
                  <option value="">(stop)</option>
                  {blockIds.map((id) => (
                    <option key={id} value={id}>
                      {id}
                    </option>
                  ))}
                </select>
              </Field>
              <Field label="Timeout (ms)">
                <input
                  type="number"
                  value={String(current.timeoutMs ?? "")}
                  onChange={(event) =>
                    patchBlock(selected, {
                      timeoutMs: event.target.value ? Number(event.target.value) : undefined,
                    })
                  }
                />
              </Field>
              <Field label="Approval">
                <select
                  value={current.requiresApproval == null ? "" : String(current.requiresApproval)}
                  onChange={(event) =>
                    patchBlock(selected, {
                      requiresApproval:
                        event.target.value === "" ? undefined : event.target.value === "true",
                    })
                  }
                >
                  <option value="">(auto)</option>
                  <option value="true">Always</option>
                  <option value="false">Never</option>
                </select>
              </Field>
            </div>
            <Field label="Block JSON">
              <textarea
                className="editor small-editor"
                spellCheck={false}
                value={blockDraft}
                onChange={(event) => setBlockDraft(event.target.value)}
              />
            </Field>
            {draftError && <div className="banner error">{draftError}</div>}
            <button className="primary" onClick={applyDraft}>
              Apply block JSON
            </button>
          </>
        ) : (
          <p className="muted">Select a block to edit it.</p>
        )}
      </div>
    </div>
  );
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="field">
      <span>{label}</span>
      {children}
    </label>
  );
}
