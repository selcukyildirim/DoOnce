import { useMemo, useState, type FormEvent } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, apiBlob, parseInput, sampleTaskSchema } from "../lib/api";
import { useToast, errorMessage } from "../lib/toast";
import { fmt, isActive, timeAgo, truncate } from "../lib/format";
import type { Task, TaskAction } from "../lib/types";
import {
  Badge,
  Button,
  Drawer,
  EmptyState,
  Field,
  PageHeader,
  ResultView,
  Section,
  Spinner,
  StatusPill,
} from "../components/ui";
import { IconPlus, IconSearch, IconSpark, IconX } from "../components/icons";

const FILTERS = ["all", "active", "completed", "failed"] as const;
type Filter = (typeof FILTERS)[number];

const PRESETS = [
  {
    label: "Google top results",
    prompt:
      'Go to Google, search for "best rust web framework" and return the top 3 organic results with title, url and snippet.',
    url: "https://www.google.com",
    schema: `{
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
  },
  "required": ["results"]
}`,
  },
  {
    label: "Google Maps places",
    prompt:
      'Search Google Maps for "coffee shops in Istanbul" and return the first 5 places with name, rating and address.',
    url: "https://www.google.com/maps",
    schema: `{
  "type": "object",
  "properties": {
    "places": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "name": { "type": "string" },
          "rating": { "type": "string" },
          "address": { "type": "string" }
        }
      }
    }
  },
  "required": ["places"]
}`,
  },
  {
    label: "YouTube search",
    prompt:
      'Search YouTube for "learn rust programming" and return the first 5 videos with title, channel, views and url.',
    url: "https://www.youtube.com",
    schema: `{
  "type": "object",
  "properties": {
    "videos": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "title": { "type": "string" },
          "channel": { "type": "string" },
          "views": { "type": "string" },
          "url": { "type": "string" }
        }
      }
    }
  },
  "required": ["videos"]
}`,
  },
  {
    label: "Sales: competitor price scan",
    prompt:
      'Search Google Shopping for "Sony WH-1000XM5 headphones". Return the top 5 offers with product title, store name, price and currency.',
    url: "https://www.google.com/search?tbm=shop&q=Sony+WH-1000XM5+headphones",
    schema: `{
  "type": "object",
  "properties": {
    "offers": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "title": { "type": "string" },
          "store": { "type": "string" },
          "price": { "type": "string" },
          "currency": { "type": "string" }
        }
      }
    }
  },
  "required": ["offers"]
}`,
  },
  {
    label: "Marketing: Hacker News trends",
    prompt:
      "Open the Hacker News front page. Extract the top 5 stories with title, points, comment count, author and URL. Then write a one-sentence summary of what developers are talking about today.",
    url: "https://news.ycombinator.com",
    schema: `{
  "type": "object",
  "properties": {
    "stories": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "title": { "type": "string" },
          "points": { "type": "string" },
          "comments": { "type": "string" },
          "author": { "type": "string" },
          "url": { "type": "string" }
        }
      }
    },
    "summary": { "type": "string" }
  },
  "required": ["stories", "summary"]
}`,
  },
  {
    label: "HR: remote Rust jobs",
    prompt:
      "Open RemoteOK and search for Rust developer jobs. Extract the first 5 job listings with title, company, location, salary (if shown) and apply URL.",
    url: "https://remoteok.com/remote-rust-jobs",
    schema: `{
  "type": "object",
  "properties": {
    "jobs": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "title": { "type": "string" },
          "company": { "type": "string" },
          "location": { "type": "string" },
          "salary": { "type": ["string","null"] },
          "url": { "type": "string" }
        }
      }
    }
  },
  "required": ["jobs"]
}`,
  },
  {
    label: "Finance: stock snapshot",
    prompt:
      "Open Yahoo Finance. Look up AAPL, MSFT and NVDA. For each, extract the current price, daily change percentage, market cap and 52-week range.",
    url: "https://finance.yahoo.com/quote/AAPL",
    schema: `{
  "type": "object",
  "properties": {
    "stocks": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "symbol": { "type": "string" },
          "price": { "type": "string" },
          "changePercent": { "type": "string" },
          "marketCap": { "type": "string" },
          "range52w": { "type": "string" }
        }
      }
    }
  },
  "required": ["stocks"]
}`,
  },
  {
    label: "Ops: service status check",
    prompt:
      "Open the GitHub Status page and the AWS Health Dashboard. Report whether each service is operational or has an active incident. Include incident titles if any.",
    url: "https://www.githubstatus.com",
    schema: `{
  "type": "object",
  "properties": {
    "services": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "service": { "type": "string" },
          "status": { "type": "string" },
          "incidents": { "type": "array", "items": { "type": "string" } }
        }
      }
    }
  },
  "required": ["services"]
}`,
  },
];

export function TasksPage() {
  const { success, error } = useToast();
  const queryClient = useQueryClient();
  const { id: routeId } = useParams();
  const navigate = useNavigate();
  const [composer, setComposer] = useState(false);
  const selected = routeId ?? null;
  const setSelected = (id: string | null) => navigate(id ? `/tasks/${id}` : "/tasks");
  const [filter, setFilter] = useState<Filter>("all");
  const [query, setQuery] = useState("");

  const [prompt, setPrompt] = useState("");
  const [url, setUrl] = useState("");
  const [schema, setSchema] = useState(sampleTaskSchema);
  const [maxSteps, setMaxSteps] = useState(30);

  const tasks = useQuery({
    queryKey: ["tasks"],
    queryFn: () => api<{ tasks: Task[] }>("/tasks?limit=100"),
    refetchInterval: (query) =>
      query.state.data?.tasks.some((task) => isActive(task.status)) ? 3000 : 15000,
  });

  const detail = useQuery({
    queryKey: ["task", selected],
    queryFn: () => api<{ task: Task }>(`/tasks/${selected}`),
    enabled: Boolean(selected),
    refetchInterval: (query) => (query.state.data?.task.status && isActive(query.state.data.task.status) ? 2500 : false),
  });

  const actions = useQuery({
    queryKey: ["task-actions", selected],
    queryFn: () => api<{ actions: TaskAction[] }>(`/tasks/${selected}/actions`),
    enabled: Boolean(selected),
    refetchInterval: isActive(detail.data?.task.status ?? "") ? 2500 : false,
  });

  const create = useMutation({
    mutationFn: () =>
      api("/tasks", {
        method: "POST",
        body: JSON.stringify({
          prompt,
          url: url || undefined,
          dataSchema: schema.trim() ? parseInput(schema, "Schema") : undefined,
          maxSteps,
        }),
      }),
    onSuccess: () => {
      success("Task queued — the worker will pick it up");
      setPrompt("");
      setSchema(sampleTaskSchema);
      setComposer(false);
      void queryClient.invalidateQueries({ queryKey: ["tasks"] });
      void queryClient.invalidateQueries({ queryKey: ["metrics"] });
    },
    onError: (reason) => error(errorMessage(reason)),
  });

  const cancel = useMutation({
    mutationFn: (id: string) => api(`/tasks/${id}/cancel`, { method: "POST" }),
    onSuccess: () => {
      success("Task cancelled");
      void queryClient.invalidateQueries({ queryKey: ["tasks"] });
      void queryClient.invalidateQueries({ queryKey: ["task", selected] });
    },
    onError: (reason) => error(errorMessage(reason)),
  });

  const rows = useMemo(() => {
    const list = tasks.data?.tasks ?? [];
    return list.filter((task) => {
      const matchesFilter =
        filter === "all"
          ? true
          : filter === "active"
            ? isActive(task.status)
            : filter === "completed"
              ? task.status === "completed"
              : ["failed", "cancelled"].includes(task.status);
      const matchesQuery =
        !query.trim() ||
        task.prompt.toLowerCase().includes(query.toLowerCase()) ||
        (task.url ?? "").toLowerCase().includes(query.toLowerCase());
      return matchesFilter && matchesQuery;
    });
  }, [tasks.data, filter, query]);

  function submit(event: FormEvent) {
    event.preventDefault();
    create.mutate();
  }

  const activeTask = detail.data?.task;

  return (
    <>
      <PageHeader
        title="Tasks"
        description="One-off AI browser jobs. Describe the outcome; DoOnce figures out the clicks."
        actions={
          <Button variant="primary" icon={<IconPlus size={16} />} onClick={() => setComposer(true)}>
            New task
          </Button>
        }
      />

      <Section>
        <div className="toolbar">
          <div className="tabs">
            {FILTERS.map((item) => (
              <button
                key={item}
                className={filter === item ? "tab active" : "tab"}
                onClick={() => setFilter(item)}
              >
                {item === "all" ? "All" : item[0].toUpperCase() + item.slice(1)}
                <span className="tab-count">
                  {tasks.data?.tasks.filter((task) =>
                    item === "all"
                      ? true
                      : item === "active"
                        ? isActive(task.status)
                        : item === "completed"
                          ? task.status === "completed"
                          : ["failed", "cancelled"].includes(task.status),
                  ).length ?? 0}
                </span>
              </button>
            ))}
          </div>
          <div className="search">
            <IconSearch size={16} />
            <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search tasks…" />
          </div>
        </div>

        {tasks.isLoading ? (
          <Spinner label="Loading tasks…" />
        ) : rows.length === 0 ? (
          <EmptyState
            icon={<IconSpark size={24} />}
            title={query || filter !== "all" ? "No matching tasks" : "No tasks yet"}
            description={
              query || filter !== "all"
                ? "Try a different search or filter."
                : "Create your first task and the worker will run it in a real browser."
            }
            action={
              <Button variant="primary" size="sm" onClick={() => setComposer(true)}>
                Create a task
              </Button>
            }
          />
        ) : (
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>Status</th>
                  <th>Prompt</th>
                  <th>Target</th>
                  <th>Result</th>
                  <th>Created</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {rows.map((task) => (
                  <tr key={task.id} className="clickable" onClick={() => setSelected(task.id)}>
                    <td>
                      <StatusPill value={task.status} />
                    </td>
                    <td className="clip">{truncate(task.prompt, 58)}</td>
                    <td className="clip muted small">{task.url ? new URL(task.url).hostname : "—"}</td>
                    <td className="clip muted small">
                      {task.output
                        ? truncate(JSON.stringify(task.output), 42)
                        : task.failureReason
                          ? truncate(task.failureReason, 42)
                          : "—"}
                    </td>
                    <td className="muted small nowrap">{fmt(task.createdAt)}</td>
                    <td onClick={(event) => event.stopPropagation()}>
                      {isActive(task.status) ? (
                        <Button
                          size="sm"
                          variant="danger"
                          icon={<IconX size={13} />}
                          onClick={() => cancel.mutate(task.id)}
                        >
                          Cancel
                        </Button>
                      ) : (
                        <Button size="sm" variant="ghost" onClick={() => setSelected(task.id)}>
                          Open
                        </Button>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Section>

      <Drawer
        open={composer}
        onClose={() => setComposer(false)}
        title="New task"
        subtitle="Describe the outcome. Add a schema if you want structured output."
      >
        <form id="task-form" className="stack" onSubmit={submit}>
          <Field label="Start from an example">
            <div className="preset-row">
              {PRESETS.map((preset) => (
                <button
                  key={preset.label}
                  type="button"
                  className="preset-chip"
                  onClick={() => {
                    setPrompt(preset.prompt);
                    setUrl(preset.url);
                    setSchema(preset.schema);
                    setMaxSteps(20);
                  }}
                >
                  {preset.label}
                </button>
              ))}
            </div>
          </Field>
          <Field label="Prompt">
            <textarea
              value={prompt}
              onChange={(event) => setPrompt(event.target.value)}
              placeholder='Go to Google, search for "best rust web framework" and return the top 3 results.'
              rows={4}
              required
            />
          </Field>
          <Field label="Start URL (optional)">
            <input value={url} onChange={(event) => setUrl(event.target.value)} placeholder="https://…" />
          </Field>
          <div className="grid2">
            <Field label="Max steps">
              <input
                type="number"
                min={1}
                max={100}
                value={maxSteps}
                onChange={(event) => setMaxSteps(Number(event.target.value))}
              />
            </Field>
            <Field label="Output schema (JSON, optional)">
              <textarea
                value={schema}
                onChange={(event) => setSchema(event.target.value)}
                rows={4}
                className="mono"
                spellCheck={false}
              />
            </Field>
          </div>
          <div className="drawer-form-actions">
            <Button type="button" variant="ghost" onClick={() => setComposer(false)}>
              Cancel
            </Button>
            <Button type="submit" variant="primary" disabled={create.isPending}>
              {create.isPending ? "Queuing…" : "Queue task"}
            </Button>
          </div>
        </form>
      </Drawer>

      <Drawer
        open={Boolean(selected)}
        onClose={() => setSelected(null)}
        title="Task detail"
        subtitle={activeTask ? <span className="mono">{activeTask.id}</span> : undefined}
        footer={
          activeTask && isActive(activeTask.status) ? (
            <Button
              variant="danger"
              icon={<IconX size={15} />}
              onClick={() => cancel.mutate(activeTask.id)}
              disabled={cancel.isPending}
            >
              Cancel task
            </Button>
          ) : undefined
        }
      >
        {!activeTask ? (
          <Spinner label="Loading task…" />
        ) : (
          <div className="stack">
            <div className="row-between">
              <StatusPill value={activeTask.status} />
              <span className="muted small">{fmt(activeTask.createdAt)}</span>
            </div>
            <div>
              <span className="field-label">Prompt</span>
              <p>{activeTask.prompt}</p>
            </div>
            {activeTask.url && (
              <div>
                <span className="field-label">Target</span>
                <p className="mono small">{activeTask.url}</p>
              </div>
            )}
            <div>
              <span className="field-label">Steps</span>
              {actions.data?.actions.length ? (
                <div className="timeline">
                  {actions.data.actions.map((action) => (
                    <div className="timeline-item" key={action.id}>
                      <span className={`timeline-dot ${action.status}`} />
                      <div className="timeline-body">
                        <div className="row-between">
                          <strong className="small">
                            {action.seq}. {summarizeAction(action.action)}
                          </strong>
                          <StatusPill value={action.status} />
                        </div>
                        {action.reasoning && <p className="muted small">{action.reasoning}</p>}
                        {action.screenshotArtifactId && (
                          <ActionShot artifactId={action.screenshotArtifactId} />
                        )}
                        <span className="muted small">{timeAgo(action.createdAt)}</span>
                      </div>
                    </div>
                  ))}
                </div>
              ) : (
                <p className="muted small">No step records yet.</p>
              )}
            </div>
            {activeTask.failureReason && (
              <div className="notice bad">
                <strong>Failure</strong>
                <p className="small">{activeTask.failureReason}</p>
              </div>
            )}
            {activeTask.output != null && (
              <div>
                <span className="field-label">Result</span>
                <ResultView value={activeTask.output} />
              </div>
            )}
            <div className="row">
              <Badge tone="muted">max {activeTask.maxSteps} steps</Badge>
            </div>
          </div>
        )}
      </Drawer>
    </>
  );
}

function summarizeAction(action: unknown): string {
  if (!action) return "step";
  if (typeof action === "string") return action;
  if (typeof action === "object") {
    const value = action as Record<string, unknown>;
    const kind = String(value.kind ?? value.type ?? "action");
    const detail = value.url ?? value.value ?? value.selector ?? value.text ?? value.target ?? "";
    return detail ? `${kind} ${String(detail)}` : kind;
  }
  return String(action);
}

function ActionShot({ artifactId }: { artifactId: string }) {
  const image = useQuery({
    queryKey: ["artifact-blob", artifactId],
    queryFn: () => apiBlob(`/artifacts/${artifactId}`),
    staleTime: Infinity,
  });
  if (!image.data) return null;
  return (
    <a className="action-shot" href={image.data} target="_blank" rel="noreferrer">
      <img src={image.data} alt="Step screenshot" />
    </a>
  );
}
