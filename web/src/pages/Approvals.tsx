import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { useToast, errorMessage } from "../lib/toast";
import { timeAgo } from "../lib/format";
import type { Approval } from "../lib/types";
import { Badge, Button, Card, EmptyState, PageHeader, Section, Spinner, StatusPill } from "../components/ui";
import { IconCheck, IconShield, IconX } from "../components/icons";

export function ApprovalsPage() {
  const { success, error } = useToast();
  const queryClient = useQueryClient();

  const approvals = useQuery({
    queryKey: ["approvals", "all"],
    queryFn: () => api<{ approvals: Approval[] }>("/approvals"),
    refetchInterval: 8000,
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

  const list = approvals.data?.approvals ?? [];
  const pending = list.filter((item) => item.status === "pending");

  return (
    <>
      <PageHeader
        title="Approvals"
        description="Human checkpoints that pause a run until someone decides."
        actions={<Badge tone={pending.length ? "warn" : "muted"}>{pending.length} pending</Badge>}
      />

      <Section title="Waiting on a human" description="Approve or deny to let the workflow continue.">
        {approvals.isLoading ? (
          <Spinner label="Loading approvals…" />
        ) : pending.length === 0 ? (
          <EmptyState icon={<IconShield size={24} />} title="Nothing pending" description="You're all caught up." />
        ) : (
          <div className="stack">
            {pending.map((approval) => (
              <Card className="approval-row" key={approval.id}>
                <div className="row-main">
                  <div className="row">
                    <StatusPill value={approval.status} />
                    <span className="mono small muted">{approval.stepId}</span>
                  </div>
                  <strong>{approval.summary}</strong>
                  <span className="muted small">
                    {approval.reason} · {timeAgo(approval.createdAt)}
                  </span>
                </div>
                <div className="row">
                  <Button
                    variant="primary"
                    icon={<IconCheck size={15} />}
                    disabled={resolve.isPending}
                    onClick={() => resolve.mutate({ id: approval.id, decision: "approved" })}
                  >
                    Approve
                  </Button>
                  <Button
                    variant="danger"
                    icon={<IconX size={15} />}
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

      <Section title="History" description="Resolved approvals.">
        <div className="table-wrap">
          <table>
            <thead>
              <tr>
                <th>Status</th>
                <th>Step</th>
                <th>Summary</th>
                <th>Resolved</th>
              </tr>
            </thead>
            <tbody>
              {list
                .filter((item) => item.status !== "pending")
                .map((approval) => (
                  <tr key={approval.id}>
                    <td>
                      <StatusPill value={approval.status} />
                    </td>
                    <td className="mono small">{approval.stepId}</td>
                    <td className="clip">{approval.summary}</td>
                    <td className="muted small">{timeAgo(approval.createdAt)}</td>
                  </tr>
                ))}
              {list.filter((item) => item.status !== "pending").length === 0 && (
                <tr>
                  <td colSpan={4} className="muted">
                    No resolved approvals yet.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </Section>
    </>
  );
}
