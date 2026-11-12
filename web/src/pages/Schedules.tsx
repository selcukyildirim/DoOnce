import { useState, type FormEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { useToast, errorMessage } from "../lib/toast";
import { fmt } from "../lib/format";
import type { Schedule, WorkflowRecord } from "../lib/types";
import {
  Badge,
  Button,
  Drawer,
  EmptyState,
  Field,
  PageHeader,
  Section,
  Spinner,
  StatusPill,
  Toggle,
} from "../components/ui";
import { IconCalendar, IconPlus, IconTrash } from "../components/icons";

export function SchedulesPage() {
  const { success, error } = useToast();
  const queryClient = useQueryClient();
  const [composer, setComposer] = useState(false);
  const [workflowId, setWorkflowId] = useState("");
  const [cron, setCron] = useState("0 9 * * *");
  const [timezone, setTimezone] = useState("UTC");

  const schedules = useQuery({
    queryKey: ["schedules"],
    queryFn: () => api<{ schedules: Schedule[] }>("/schedules"),
  });

  const workflows = useQuery({
    queryKey: ["workflows"],
    queryFn: () => api<{ workflows: WorkflowRecord[] }>("/workflows"),
  });

  const create = useMutation({
    mutationFn: () =>
      api("/schedules", {
        method: "POST",
        body: JSON.stringify({ workflowId, cron, timezone }),
      }),
    onSuccess: () => {
      success("Schedule created");
      setComposer(false);
      setWorkflowId("");
      void queryClient.invalidateQueries({ queryKey: ["schedules"] });
    },
    onError: (reason) => error(errorMessage(reason)),
  });

  const toggle = useMutation({
    mutationFn: (schedule: Schedule) =>
      api(`/schedules/${schedule.id}/enabled`, {
        method: "PATCH",
        body: JSON.stringify({ enabled: !schedule.enabled }),
      }),
    onSuccess: () => void queryClient.invalidateQueries({ queryKey: ["schedules"] }),
    onError: (reason) => error(errorMessage(reason)),
  });

  const remove = useMutation({
    mutationFn: (id: string) => api(`/schedules/${id}`, { method: "DELETE" }),
    onSuccess: () => {
      success("Schedule deleted");
      void queryClient.invalidateQueries({ queryKey: ["schedules"] });
    },
    onError: (reason) => error(errorMessage(reason)),
  });

  const list = schedules.data?.schedules ?? [];
  const workflowList = workflows.data?.workflows ?? [];
  const nameFor = (id: string) => workflowList.find((workflow) => workflow.id === id)?.name ?? id.slice(0, 8);

  function submit(event: FormEvent) {
    event.preventDefault();
    create.mutate();
  }

  return (
    <>
      <PageHeader
        title="Schedules"
        description="Run workflows on a cron cadence. Pause or delete any schedule instantly."
        actions={
          <Button variant="primary" icon={<IconPlus size={16} />} onClick={() => setComposer(true)}>
            New schedule
          </Button>
        }
      />

      <Section
        title={`Active schedules (${list.length})`}
        description="Times are shown in each schedule's own timezone."
      >
        {schedules.isLoading ? (
          <Spinner label="Loading schedules…" />
        ) : list.length === 0 ? (
          <EmptyState
            icon={<IconCalendar size={24} />}
            title="No schedules yet"
            description="Pick a workflow and a cron expression to automate it."
            action={
              <Button variant="primary" size="sm" onClick={() => setComposer(true)}>
                Create a schedule
              </Button>
            }
          />
        ) : (
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>Enabled</th>
                  <th>Workflow</th>
                  <th>Cron</th>
                  <th>Timezone</th>
                  <th>Next run</th>
                  <th>Last status</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {list.map((schedule) => (
                  <tr key={schedule.id}>
                    <td>
                      <Toggle
                        checked={schedule.enabled}
                        onChange={() => toggle.mutate(schedule)}
                        label="Toggle schedule"
                      />
                    </td>
                    <td>
                      <strong>{nameFor(schedule.workflowId)}</strong>
                    </td>
                    <td className="mono small">{schedule.cron}</td>
                    <td className="muted small">{schedule.timezone}</td>
                    <td className="muted small nowrap">{schedule.nextRunAt ? fmt(schedule.nextRunAt) : "—"}</td>
                    <td>{schedule.lastStatus ? <StatusPill value={schedule.lastStatus} /> : <span className="muted">—</span>}</td>
                    <td>
                      <Button
                        size="sm"
                        variant="ghost"
                        icon={<IconTrash size={14} />}
                        onClick={() => remove.mutate(schedule.id)}
                      >
                        Delete
                      </Button>
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
        title="New schedule"
        subtitle="Cron uses the standard five-field syntax."
      >
        <form className="stack" onSubmit={submit}>
          <Field label="Workflow">
            <select value={workflowId} onChange={(event) => setWorkflowId(event.target.value)} required>
              <option value="">Select a workflow…</option>
              {workflowList.map((workflow) => (
                <option key={workflow.id} value={workflow.id}>
                  {workflow.name}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Cron expression" hint="Example: 0 9 * * * runs every day at 09:00.">
            <input value={cron} onChange={(event) => setCron(event.target.value)} className="mono" required />
          </Field>
          <Field label="Timezone" hint="IANA name such as Europe/Istanbul.">
            <input value={timezone} onChange={(event) => setTimezone(event.target.value)} required />
          </Field>
          <div className="drawer-form-actions">
            <Button type="button" variant="ghost" onClick={() => setComposer(false)}>
              Cancel
            </Button>
            <Button type="submit" variant="primary" disabled={create.isPending || !workflowId}>
              Create schedule
            </Button>
          </div>
          <Badge tone="muted">Tip: enable a workflow first if it depends on credentials or approvals.</Badge>
        </form>
      </Drawer>
    </>
  );
}
