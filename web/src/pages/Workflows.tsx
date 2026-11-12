import { useEffect, useState, type FormEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, parseInput, pretty, sampleWorkflow } from "../lib/api";
import { useToast, errorMessage } from "../lib/toast";
import { fmt, timeAgo } from "../lib/format";
import type { Run, WorkflowDetail, WorkflowRecord } from "../lib/types";
import { WorkflowBuilder } from "../WorkflowBuilder";
import {
  Badge,
  Button,
  Card,
  Drawer,
  EmptyState,
  Field,
  PageHeader,
  Section,
  Spinner,
  StatusPill,
} from "../components/ui";
import {
  IconPlus,
  IconPlay,
  IconSpark,
  IconTerminal,
  IconTrash,
  IconWorkflow,
} from "../components/icons";

export function WorkflowsPage() {
  const { success, error } = useToast();
  const queryClient = useQueryClient();
  const [composer, setComposer] = useState(false);
  const [editor, setEditor] = useState<{ slug: string | null } | null>(null);
  const [json, setJson] = useState(sampleWorkflow);
  const [builderMode, setBuilderMode] = useState(false);
  const [validation, setValidation] = useState<string | null>(null);
  const [generateOpen, setGenerateOpen] = useState(false);
  const [generateDesc, setGenerateDesc] = useState("");

  const [name, setName] = useState("");
  const [prompt, setPrompt] = useState("");
  const [url, setUrl] = useState("");
  const [schema, setSchema] = useState("");

  const workflows = useQuery({
    queryKey: ["workflows"],
    queryFn: () => api<{ workflows: WorkflowRecord[] }>("/workflows"),
  });

  const detail = useQuery({
    queryKey: ["workflow", editor?.slug],
    queryFn: () => api<WorkflowDetail>(`/workflows/${encodeURIComponent(editor?.slug ?? "")}`),
    enabled: Boolean(editor?.slug),
  });

  useEffect(() => {
    if (detail.data && editor) {
      setJson(pretty(detail.data.definition));
    }
  }, [detail.data, editor]);

  const createPrompt = useMutation({
    mutationFn: async (runNow: boolean) => {
      const definition: Record<string, unknown> = {
        schemaVersion: "2.0",
        name,
        prompt,
        blocks: [],
      };
      if (url.trim()) definition.url = url.trim();
      if (schema.trim()) definition.dataSchema = parseInput(schema, "Schema");
      const created = await api<{ workflow: WorkflowRecord }>("/workflows", {
        method: "POST",
        body: JSON.stringify({ definition, reason: "Prompt workflow from console" }),
      });
      if (runNow) {
        await api(`/workflows/${encodeURIComponent(created.workflow.slug)}/runs`, {
          method: "POST",
          body: JSON.stringify({ inputs: {} }),
        });
      }
      return { slug: created.workflow.slug, runNow };
    },
    onSuccess: ({ runNow }) => {
      success(runNow ? "Workflow created and queued" : "Workflow created");
      setName("");
      setPrompt("");
      setUrl("");
      setSchema("");
      setComposer(false);
      void queryClient.invalidateQueries({ queryKey: ["workflows"] });
      void queryClient.invalidateQueries({ queryKey: ["runs"] });
    },
    onError: (reason) => error(errorMessage(reason)),
  });

  const runWorkflow = useMutation({
    mutationFn: (slug: string) =>
      api(`/workflows/${encodeURIComponent(slug)}/runs`, { method: "POST", body: JSON.stringify({ inputs: {} }) }),
    onSuccess: () => {
      success("Run queued");
      void queryClient.invalidateQueries({ queryKey: ["runs"] });
      void queryClient.invalidateQueries({ queryKey: ["metrics"] });
    },
    onError: (reason) => error(errorMessage(reason)),
  });

  const remove = useMutation({
    mutationFn: (slug: string) => api(`/workflows/${encodeURIComponent(slug)}`, { method: "DELETE" }),
    onSuccess: () => {
      success("Workflow deleted");
      void queryClient.invalidateQueries({ queryKey: ["workflows"] });
    },
    onError: (reason) => error(errorMessage(reason)),
  });

  const save = useMutation({
    mutationFn: async () => {
      const definition = parseInput(json, "Workflow");
      if (editor?.slug) {
        await api(`/workflows/${encodeURIComponent(editor.slug)}`, {
          method: "PUT",
          body: JSON.stringify({ definition, reason: "Updated from console" }),
        });
        return "version";
      }
      await api("/workflows", {
        method: "POST",
        body: JSON.stringify({ definition, reason: "Created from console" }),
      });
      return "created";
    },
    onSuccess: (kind) => {
      success(kind === "version" ? "New workflow version saved" : "Workflow created");
      setEditor(null);
      void queryClient.invalidateQueries({ queryKey: ["workflows"] });
    },
    onError: (reason) => error(errorMessage(reason)),
  });

  const validate = useMutation({
    mutationFn: async () => {
      const definition = parseInput(json, "Workflow");
      return api<{ valid: boolean; issues: { path: string; message: string }[] }>("/workflows/validate", {
        method: "POST",
        body: JSON.stringify(definition),
      });
    },
    onSuccess: (result) => {
      setValidation(
        result.valid ? "Workflow is valid." : result.issues.map((issue) => `${issue.path}: ${issue.message}`).join("\n"),
      );
    },
    onError: (reason) => error(errorMessage(reason)),
  });

  const generate = useMutation({
    mutationFn: async () => {
      const result = await api<{ definition: unknown }>("/workflows/generate", {
        method: "POST",
        body: JSON.stringify({ description: generateDesc }),
      });
      return result.definition;
    },
    onSuccess: (definition) => {
      setJson(pretty(definition));
      setEditor({ slug: null });
      setGenerateOpen(false);
      setGenerateDesc("");
      success("Workflow generated — review and save");
    },
    onError: (reason) => error(errorMessage(reason)),
  });

  const list = workflows.data?.workflows ?? [];

  function submitPrompt(event: FormEvent) {
    event.preventDefault();
    createPrompt.mutate(false);
  }

  return (
    <>
      <PageHeader
        title="Workflows"
        description="Reusable automations. Start with a prompt, or open the builder for full control."
        actions={
          <>
            <Button
              variant="secondary"
              icon={<IconSpark size={16} />}
              onClick={() => setGenerateOpen(true)}
            >
              Generate from description
            </Button>
            <Button
              variant="secondary"
              icon={<IconTerminal size={16} />}
              onClick={() => {
                setJson(sampleWorkflow);
                setBuilderMode(false);
                setValidation(null);
                setEditor({ slug: null });
              }}
            >
              JSON editor
            </Button>
            <Button variant="primary" icon={<IconPlus size={16} />} onClick={() => setComposer(true)}>
              New workflow
            </Button>
          </>
        }
      />

      <Card className="prompt-card">
        <div>
          <Badge tone="accent">
            <IconSpark size={13} /> simplest path
          </Badge>
          <h2>Describe it. Save it. Schedule it.</h2>
          <p className="muted">
            A prompt workflow is one AI browser task wrapped as a real, versioned workflow — so it can be rerun,
            scheduled and audited.
          </p>
        </div>
        <Button variant="primary" onClick={() => setComposer(true)}>
          Create from a prompt
        </Button>
      </Card>

      <Section
        title={`All workflows (${list.length})`}
        description="Every workflow keeps a version history and run log."
        actions={<Badge tone="muted">{list.length} total</Badge>}
      >
        {workflows.isLoading ? (
          <Spinner label="Loading workflows…" />
        ) : list.length === 0 ? (
          <EmptyState
            icon={<IconWorkflow size={24} />}
            title="No workflows yet"
            description="Create one from a prompt in under a minute."
            action={
              <Button variant="primary" size="sm" onClick={() => setComposer(true)}>
                New workflow
              </Button>
            }
          />
        ) : (
          <div className="workflow-grid">
            {list.map((workflow) => (
              <article className="workflow-card" key={workflow.id}>
                <div className="workflow-card-top">
                  <span className="workflow-glyph">
                    <IconWorkflow size={20} />
                  </span>
                  <Badge tone="muted">v{workflow.currentVersion}</Badge>
                </div>
                <h3>{workflow.name}</h3>
                <p className="mono small muted">{workflow.slug}</p>
                <div className="workflow-meta">
                  <span className="muted small">updated {timeAgo(workflow.updatedAt)}</span>
                </div>
                <div className="workflow-actions">
                  <Button
                    size="sm"
                    variant="primary"
                    icon={<IconPlay size={14} />}
                    onClick={() => runWorkflow.mutate(workflow.slug)}
                    disabled={runWorkflow.isPending}
                  >
                    Run
                  </Button>
                  <Button
                    size="sm"
                    variant="secondary"
                    onClick={() => {
                      setValidation(null);
                      setBuilderMode(false);
                      setEditor({ slug: workflow.slug });
                    }}
                  >
                    Edit
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    icon={<IconTrash size={14} />}
                    onClick={() => {
                      if (confirm(`Delete ${workflow.slug} and its history?`)) remove.mutate(workflow.slug);
                    }}
                  >
                    Delete
                  </Button>
                </div>
              </article>
            ))}
          </div>
        )}
      </Section>

      <Drawer
        open={composer}
        onClose={() => setComposer(false)}
        title="Prompt workflow"
        subtitle="One prompt becomes a versioned, schedulable workflow."
      >
        <form className="stack" onSubmit={submitPrompt}>
          <Field label="Workflow name">
            <input value={name} onChange={(event) => setName(event.target.value)} placeholder="Google search digest" required />
          </Field>
          <Field label="Prompt">
            <textarea
              value={prompt}
              onChange={(event) => setPrompt(event.target.value)}
              rows={4}
              placeholder='Go to Google, search for "best rust web framework" and return the top 3 results with title, url and snippet.'
              required
            />
          </Field>
          <Field label="Start URL (optional)">
            <input value={url} onChange={(event) => setUrl(event.target.value)} placeholder="https://www.google.com" />
          </Field>
          <Field label="Output schema (JSON, optional)">
            <textarea
              value={schema}
              onChange={(event) => setSchema(event.target.value)}
              rows={4}
              className="mono"
              spellCheck={false}
              placeholder='{"type":"object","properties":{"items":{"type":"array"}}}'
            />
          </Field>
          <div className="drawer-form-actions">
            <Button type="button" variant="ghost" onClick={() => setComposer(false)}>
              Cancel
            </Button>
            <Button
              type="button"
              variant="secondary"
              onClick={() => createPrompt.mutate(true)}
              disabled={createPrompt.isPending || !name.trim() || !prompt.trim()}
            >
              Create &amp; run
            </Button>
            <Button type="submit" variant="primary" disabled={createPrompt.isPending}>
              Create
            </Button>
          </div>
        </form>
      </Drawer>

      <Drawer
        open={Boolean(editor)}
        onClose={() => setEditor(null)}
        title={editor?.slug ? `Edit ${editor.slug}` : "New workflow"}
        subtitle="Switch between the visual builder and raw JSON."
        footer={
          <>
            <Button variant="ghost" onClick={() => setEditor(null)}>
              Cancel
            </Button>
            <Button variant="secondary" onClick={() => validate.mutate()} disabled={validate.isPending}>
              Validate
            </Button>
            <Button variant="primary" onClick={() => save.mutate()} disabled={save.isPending}>
              {editor?.slug ? "Save version" : "Create"}
            </Button>
          </>
        }
      >
        {editor?.slug && detail.isLoading ? (
          <Spinner label="Loading definition…" />
        ) : (
          <div className="stack">
            <div className="tabs">
              <button className={!builderMode ? "tab active" : "tab"} onClick={() => setBuilderMode(false)}>
                JSON
              </button>
              <button className={builderMode ? "tab active" : "tab"} onClick={() => setBuilderMode(true)}>
                Builder
              </button>
            </div>
            {validation && (
              <div className={validation === "Workflow is valid." ? "notice ok" : "notice bad"}>
                <pre className="code small">{validation}</pre>
              </div>
            )}
            {builderMode ? (
              <WorkflowBuilder json={json} onChange={setJson} />
            ) : (
              <textarea
                className="editor mono"
                spellCheck={false}
                rows={22}
                value={json}
                onChange={(event) => setJson(event.target.value)}
              />
            )}
            {editor?.slug && detail.data && (
              <p className="muted small">
                Current version v{detail.data.workflow.currentVersion} · last updated{" "}
                {fmt(detail.data.workflow.updatedAt)}
              </p>
            )}
          </div>
        )}
      </Drawer>

      <Drawer
        open={generateOpen}
        onClose={() => setGenerateOpen(false)}
        title="Generate workflow from description"
        subtitle="Describe what you want in plain English. AI builds a v2 workflow."
        footer={
          <>
            <Button variant="ghost" onClick={() => setGenerateOpen(false)}>Cancel</Button>
            <Button variant="primary" onClick={() => generate.mutate()} disabled={generate.isPending || !generateDesc.trim()}>
              {generate.isPending ? "Generating…" : "Generate"}
            </Button>
          </>
        }
      >
        <div className="stack">
          <Field label="Description" hint="Be specific: sites, data to extract, conditions, actions.">
            <textarea
              value={generateDesc}
              onChange={(e) => setGenerateDesc(e.target.value)}
              rows={6}
              placeholder='Every morning at 09:00, open example.com, check if the price changed, and email me if it did.'
            />
          </Field>
          <p className="muted small">
            The generated workflow opens in the JSON editor for review. Validate and save before running.
          </p>
        </div>
      </Drawer>

      <RunReference />
    </>
  );
}

function RunReference() {
  const runs = useQuery({
    queryKey: ["runs", "workflow-latest"],
    queryFn: () => api<{ runs: Run[] }>("/runs?limit=5"),
    refetchInterval: 8000,
  });
  if (!runs.data || runs.data.runs.length === 0) return null;
  return (
    <Section title="Latest runs" description="Jump straight into the last executions.">
      <div className="row-list">
        {runs.data.runs.map((run) => (
          <div className="row-item" key={run.id}>
            <div className="row-main">
              <StatusPill value={run.status} />
              <span className="mono small">{run.id.slice(0, 10)}</span>
            </div>
            <span className="muted small">{timeAgo(run.createdAt)}</span>
          </div>
        ))}
      </div>
    </Section>
  );
}
