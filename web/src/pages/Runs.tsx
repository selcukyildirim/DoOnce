import { useMemo, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { api, apiBlob } from "../lib/api";
import { duration, fmt, isActive, timeAgo } from "../lib/format";
import type { Artifact, BlockRun, Run } from "../lib/types";
import {
  Badge,
  Button,
  Drawer,
  EmptyState,
  PageHeader,
  ResultView,
  Section,
  Spinner,
  StatusPill,
} from "../components/ui";
import { IconImage, IconRepo, IconRuns, IconSearch } from "../components/icons";

export function RunsPage() {
  const { id } = useParams();
  const navigate = useNavigate();
  const [query, setQuery] = useState("");
  const [status, setStatus] = useState("all");

  const runs = useQuery({
    queryKey: ["runs"],
    queryFn: () => api<{ runs: Run[] }>("/runs?limit=100"),
    refetchInterval: (q) => (q.state.data?.runs.some((run) => isActive(run.status)) ? 3000 : 15000),
  });

  const detail = useQuery({
    queryKey: ["run", id],
    queryFn: () => api<{ run: Run; blocks: BlockRun[] }>(`/runs/${id}`),
    enabled: Boolean(id),
    refetchInterval: (q) => (q.state.data?.run.status && isActive(q.state.data.run.status) ? 2500 : false),
  });

  const artifacts = useQuery({
    queryKey: ["run-artifacts", id],
    queryFn: () => api<{ artifacts: Artifact[] }>(`/runs/${id}/artifacts`),
    enabled: Boolean(id),
  });

  const rows = useMemo(() => {
    const list = runs.data?.runs ?? [];
    return list.filter((run) => {
      const matchesStatus = status === "all" || run.status === status;
      const matchesQuery = !query.trim() || run.id.toLowerCase().includes(query.toLowerCase());
      return matchesStatus && matchesQuery;
    });
  }, [runs.data, query, status]);

  const active = detail.data;
  const screenshots = (artifacts.data?.artifacts ?? []).filter((artifact) => artifact.kind === "screenshot");

  return (
    <>
      <PageHeader
        title="Runs"
        description="Every workflow execution, its blocks, outputs and captured screenshots."
        actions={<Badge tone="muted">{runs.data?.runs.length ?? 0} runs</Badge>}
      />

      <Section>
        <div className="toolbar">
          <div className="tabs">
            {["all", "running", "completed", "failed"].map((item) => (
              <button key={item} className={status === item ? "tab active" : "tab"} onClick={() => setStatus(item)}>
                {item === "all" ? "All" : item[0].toUpperCase() + item.slice(1)}
              </button>
            ))}
          </div>
          <div className="search">
            <IconSearch size={16} />
            <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search by run id…" />
          </div>
        </div>

        {runs.isLoading ? (
          <Spinner label="Loading runs…" />
        ) : rows.length === 0 ? (
          <EmptyState icon={<IconRuns size={24} />} title="No runs match" description="Adjust the filter or start a workflow." />
        ) : (
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>Status</th>
                  <th>Run</th>
                  <th>Trigger</th>
                  <th>Started</th>
                  <th>Finished</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {rows.map((run) => (
                  <tr key={run.id} className="clickable" onClick={() => navigate(`/runs/${run.id}`)}>
                    <td>
                      <StatusPill value={run.status} />
                    </td>
                    <td className="mono small">{run.id.slice(0, 12)}</td>
                    <td>
                      <Badge tone="muted">{run.trigger}</Badge>
                    </td>
                    <td className="muted small nowrap">{timeAgo(run.createdAt)}</td>
                    <td className="muted small nowrap">{run.finishedAt ? timeAgo(run.finishedAt) : "—"}</td>
                    <td>
                      <Button size="sm" variant="ghost" onClick={() => navigate(`/runs/${run.id}`)}>
                        Open
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
        open={Boolean(id)}
        onClose={() => navigate("/runs")}
        title="Run detail"
        subtitle={id ? <span className="mono">{id}</span> : undefined}
        footer={
          id ? (
            <Button
              variant="secondary"
              icon={<IconRepo size={15} />}
              onClick={async () => {
                try {
                  const blob = await fetch(`/api/v1/runs/${id}/evidence`, { credentials: "same-origin" }).then((r) => r.blob());
                  const url = URL.createObjectURL(blob);
                  const a = document.createElement("a");
                  a.href = url;
                  a.download = `evidence-${id}.zip`;
                  a.click();
                  URL.revokeObjectURL(url);
                } catch (e) {
                  console.error(e);
                }
              }}
            >
              Download evidence (ZIP)
            </Button>
          ) : undefined
        }
      >
        {detail.isLoading || !active ? (
          <Spinner label="Loading run…" />
        ) : (
          <div className="stack">
            <div className="row-between">
              <StatusPill value={active.run.status} />
              <span className="muted small">
                {active.run.trigger} · {duration(
                  active.run.finishedAt
                    ? new Date(active.run.finishedAt).getTime() - new Date(active.run.createdAt).getTime()
                    : null,
                )}
              </span>
            </div>

            {active.run.error && (
              <div className="notice bad">
                <strong>Run error</strong>
                <p className="small">{active.run.error}</p>
              </div>
            )}

            <div>
              <span className="field-label">Blocks</span>
              <div className="timeline">
                {active.blocks.length === 0 && <p className="muted small">No block records yet.</p>}
                {active.blocks.map((block) => (
                  <div className="timeline-item" key={block.id}>
                    <span className={`timeline-dot ${block.status}`} />
                    <div className="timeline-body">
                      <div className="row-between">
                        <strong className="mono small">{block.blockId}</strong>
                        <StatusPill value={block.status} />
                      </div>
                      {block.error && <p className="small bad-text">{block.error}</p>}
                      <span className="muted small">{block.finishedAt ? fmt(block.finishedAt) : "in progress"}</span>
                    </div>
                  </div>
                ))}
              </div>
            </div>

            <div>
              <span className="field-label">Outputs</span>
              {active.run.outputs != null ? (
                <ResultView value={active.run.outputs} />
              ) : (
                <p className="muted small">No outputs recorded.</p>
              )}
            </div>

            <div>
              <span className="field-label">Screenshots</span>
              {artifacts.isLoading ? (
                <Spinner />
              ) : screenshots.length === 0 ? (
                <p className="muted small">No screenshots captured for this run.</p>
              ) : (
                <div className="shot-grid">
                  {screenshots.map((artifact) => (
                    <Shot key={artifact.id} artifact={artifact} />
                  ))}
                </div>
              )}
            </div>

            <div>
              <span className="field-label">Artifacts</span>
              <div className="row-list">
                {(artifacts.data?.artifacts ?? []).map((artifact) => (
                  <div className="row-item" key={artifact.id}>
                    <div className="row-main">
                      <IconImage size={16} />
                      <span className="mono small">{artifact.filename}</span>
                    </div>
                    <span className="muted small">{artifact.kind}</span>
                  </div>
                ))}
                {(artifacts.data?.artifacts ?? []).length === 0 && <p className="muted small">None.</p>}
              </div>
            </div>
          </div>
        )}
      </Drawer>
    </>
  );
}

function Shot({ artifact }: { artifact: Artifact }) {
  const image = useQuery({
    queryKey: ["artifact-blob", artifact.id],
    queryFn: () => apiBlob(`/artifacts/${artifact.id}`),
    staleTime: Infinity,
  });
  return (
    <figure className="shot">
      {image.data ? <img src={image.data} alt={artifact.filename} /> : <div className="skeleton shot-skeleton" />}
      <figcaption className="muted small">{artifact.filename}</figcaption>
    </figure>
  );
}
