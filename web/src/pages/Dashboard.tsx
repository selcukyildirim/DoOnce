import { Link } from "react-router-dom";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { useSession } from "../lib/session";
import { useToast, errorMessage } from "../lib/toast";
import { duration, fmt, isActive, timeAgo, truncate } from "../lib/format";
import type { Approval, Metrics, Run, Task } from "../lib/types";
import { Badge, Button, Card, EmptyState, PageHeader, Section, StatCard, StatusPill } from "../components/ui";
import {
  IconArrowUpRight,
  IconCheck,
  IconClock,
  IconPlay,
  IconRuns,
  IconShield,
  IconSpark,
  IconWorkflow,
  IconX,
  IconZap,
} from "../components/icons";

export function DashboardPage() {
  const { principal } = useSession();
  const { success, error } = useToast();
  const queryClient = useQueryClient();

  const metrics = useQuery({
    queryKey: ["metrics"],
    queryFn: () => api<{ metrics: Metrics }>("/metrics"),
    refetchInterval: 10000,
  });
  const runs = useQuery({
    queryKey: ["runs"],
    queryFn: () => api<{ runs: Run[] }>("/runs?limit=100"),
    refetchInterval: 5000,
  });
  const tasks = useQuery({
    queryKey: ["tasks"],
    queryFn: () => api<{ tasks: Task[] }>("/tasks?limit=100"),
    refetchInterval: 5000,
  });
  const approvals = useQuery({
    queryKey: ["approvals", "pending"],
    queryFn: () => api<{ approvals: Approval[] }>("/approvals?status=pending"),
    refetchInterval: 10000,
  });

  const resolve = useMutation({
    mutationFn: ({ id, decision }: { id: string; decision: "approved" | "denied" }) =>
      api(`/approvals/${id}`, { method: "POST", body: JSON.stringify({ decision }) }),
    onSuccess: (_, vars) => {
      success(vars.decision === "approved" ? "Approval granted" : "Approval denied");
      void queryClient.invalidateQueries({ queryKey: ["approvals"] });
      void queryClient.invalidateQueries({ queryKey: ["metrics"] });
    },
    onError: (reason) => error(errorMessage(reason)),
  });

  const stats = metrics.data?.metrics;
  const runList = runs.data?.runs ?? [];
  const taskList = tasks.data?.tasks ?? [];
  const pending = approvals.data?.approvals ?? [];
  const activeRuns = runList.filter((run) => isActive(run.status));
  const activeTasks = taskList.filter((task) => isActive(task.status));

  return (
    <>
      <PageHeader
        title={`Welcome back, ${principal?.tenant.name ?? "team"}`}
        description="Live view of everything DoOnce is doing across your workspace."
        actions={
          <>
            <Link to="/tasks">
              <Button variant="secondary" icon={<IconSpark size={16} />}>
                New task
              </Button>
            </Link>
            <Link to="/workflows">
              <Button variant="primary" icon={<IconPlay size={16} />}>
                Run a workflow
              </Button>
            </Link>
          </>
        }
      />

      <div className="hero-strip">
        <div className="hero-copy">
          <Badge tone="accent">
            <IconZap size={13} /> live workspace
          </Badge>
          <h2>
            {activeRuns.length + activeTasks.length > 0
              ? `${activeRuns.length + activeTasks.length} job${activeRuns.length + activeTasks.length === 1 ? "" : "s"} in flight`
              : "Everything is idle"}
          </h2>
          <p className="muted">
            {activeRuns.length + activeTasks.length > 0
              ? "Results appear here as soon as the worker reports back."
              : "Queue a task or a workflow and watch it work in a real browser."}
          </p>
        </div>
        <div className="hero-meter">
          <div className="meter">
            <span style={{ width: `${Math.min(100, stats?.successRate ?? 0)}%` }} />
          </div>
          <div className="meter-label">
            <span>Success rate</span>
            <strong>{stats ? `${stats.successRate.toFixed(1)}%` : "—"}</strong>
          </div>
        </div>
      </div>

      <div className="stat-grid">
        <StatCard
          label="Workflows"
          value={stats?.workflows ?? "—"}
          hint="reusable automations"
          icon={<IconWorkflow size={18} />}
        />
        <StatCard label="Tasks" value={stats?.tasks ?? "—"} hint="one-off browser jobs" icon={<IconSpark size={18} />} />
        <StatCard
          label="Runs completed"
          value={stats?.runsCompleted ?? "—"}
          hint={`${stats?.runsFailed ?? 0} failed`}
          tone="ok"
          icon={<IconCheck size={18} />}
        />
        <StatCard
          label="Queue depth"
          value={stats ? stats.runsQueued + stats.tasksQueued : "—"}
          hint="waiting for a worker"
          tone="warn"
          icon={<IconClock size={18} />}
        />
        <StatCard
          label="Avg run time"
          value={duration(stats?.avgRunDurationMs)}
          hint="end-to-end"
          icon={<IconRuns size={18} />}
        />
        <StatCard
          label="Pending approvals"
          value={pending.length}
          hint="needs a human"
          tone={pending.length > 0 ? "warn" : undefined}
          icon={<IconShield size={18} />}
        />
      </div>

      <div className="two-col">
        <Section
          title="Recent runs"
          description="The latest workflow executions."
          actions={
            <Link to="/runs" className="link-inline">
              View all <IconArrowUpRight size={14} />
            </Link>
          }
        >
          {runs.isLoading ? (
            <div className="row-list">
              {[0, 1, 2].map((key) => (
                <div className="row-item" key={key}>
                  <div className="skeleton" style={{ width: "40%" }} />
                  <div className="skeleton" style={{ width: "18%" }} />
                </div>
              ))}
            </div>
          ) : runList.length === 0 ? (
            <EmptyState
              icon={<IconRuns size={22} />}
              title="No runs yet"
              description="Start a workflow to see execution history here."
              action={
                <Link to="/workflows">
                  <Button variant="secondary" size="sm">
                    Go to workflows
                  </Button>
                </Link>
              }
            />
          ) : (
            <div className="row-list">
              {runList.slice(0, 6).map((run) => (
                <Link to={`/runs/${run.id}`} className="row-item row-link" key={run.id}>
                  <div className="row-main">
                    <StatusPill value={run.status} />
                    <span className="mono small">{run.id.slice(0, 10)}</span>
                  </div>
                  <div className="row-meta">
                    <span className="muted small">{run.trigger}</span>
                    <span className="muted small">{timeAgo(run.createdAt)}</span>
                  </div>
                </Link>
              ))}
            </div>
          )}
        </Section>

        <Section
          title="Needs you"
          description="Human approvals blocking a run."
          actions={<Badge tone={pending.length ? "warn" : "muted"}>{pending.length} pending</Badge>}
        >
          {pending.length === 0 ? (
            <EmptyState icon={<IconCheck size={22} />} title="Nothing waiting" description="Approvals will surface here." />
          ) : (
            <div className="stack">
              {pending.slice(0, 4).map((approval) => (
                <Card className="approval-card" key={approval.id}>
                  <div className="row-main">
                    <strong>{approval.summary}</strong>
                    <span className="muted small">{approval.reason}</span>
                  </div>
                  <div className="row">
                    <Button
                      size="sm"
                      variant="primary"
                      icon={<IconCheck size={14} />}
                      disabled={resolve.isPending}
                      onClick={() => resolve.mutate({ id: approval.id, decision: "approved" })}
                    >
                      Approve
                    </Button>
                    <Button
                      size="sm"
                      variant="danger"
                      icon={<IconX size={14} />}
                      disabled={resolve.isPending}
                      onClick={() => resolve.mutate({ id: approval.id, decision: "denied" })}
                    >
                      Deny
                    </Button>
                  </div>
                </Card>
              ))}
            </div>
          )}
        </Section>
      </div>

      <Section title="Recent tasks" description="One-off AI browser jobs and their latest result.">
        {taskList.length === 0 ? (
          <EmptyState
            icon={<IconSpark size={22} />}
            title="No tasks yet"
            description="Describe a job and DoOnce will drive the browser."
            action={
              <Link to="/tasks">
                <Button variant="secondary" size="sm">
                  Create a task
                </Button>
              </Link>
            }
          />
        ) : (
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>Status</th>
                  <th>Prompt</th>
                  <th>Result</th>
                  <th>Created</th>
                </tr>
              </thead>
              <tbody>
                {taskList.slice(0, 6).map((task) => (
                  <tr key={task.id}>
                    <td>
                      <StatusPill value={task.status} />
                    </td>
                    <td className="clip">{truncate(task.prompt, 70)}</td>
                    <td className="clip muted">
                      {task.output ? truncate(JSON.stringify(task.output), 48) : task.failureReason ?? "—"}
                    </td>
                    <td className="muted small nowrap">{fmt(task.createdAt)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Section>
    </>
  );
}
