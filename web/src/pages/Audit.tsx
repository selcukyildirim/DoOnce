import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "../lib/api";
import { fmt } from "../lib/format";
import type { AuditLog } from "../lib/types";
import { Badge, EmptyState, PageHeader, Section, Spinner } from "../components/ui";
import { IconRepo, IconSearch } from "../components/icons";

export function AuditPage() {
  const [query, setQuery] = useState("");
  const audit = useQuery({
    queryKey: ["audit"],
    queryFn: () => api<{ audit: AuditLog[] }>("/audit?limit=100"),
  });

  const rows = useMemo(() => {
    const list = audit.data?.audit ?? [];
    if (!query.trim()) return list;
    return list.filter(
      (entry) =>
        entry.action.toLowerCase().includes(query.toLowerCase()) ||
        entry.target.toLowerCase().includes(query.toLowerCase()) ||
        (entry.actorId ?? "").toLowerCase().includes(query.toLowerCase()),
    );
  }, [audit.data, query]);

  return (
    <>
      <PageHeader
        title="Audit log"
        description="An append-only record of sensitive actions across the workspace."
        actions={<Badge tone="muted">{audit.data?.audit.length ?? 0} events</Badge>}
      />

      <Section>
        <div className="toolbar">
          <span className="muted small">Newest first</span>
          <div className="search">
            <IconSearch size={16} />
            <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search action, target, actor…" />
          </div>
        </div>

        {audit.isLoading ? (
          <Spinner label="Loading audit log…" />
        ) : rows.length === 0 ? (
          <EmptyState icon={<IconRepo size={24} />} title="No matching events" description="Try a different search." />
        ) : (
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>When</th>
                  <th>Action</th>
                  <th>Target</th>
                  <th>Actor</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((entry) => (
                  <tr key={entry.id}>
                    <td className="muted small nowrap">{fmt(entry.createdAt)}</td>
                    <td>
                      <span className="mono small">{entry.action}</span>
                    </td>
                    <td className="clip small">{entry.target}</td>
                    <td className="muted small mono">{entry.actorId ? entry.actorId.slice(0, 12) : "system"}</td>
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
