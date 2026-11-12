import { useState, type FormEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { useSession } from "../lib/session";
import { useToast, errorMessage } from "../lib/toast";
import { timeAgo } from "../lib/format";
import type { Connector } from "../lib/types";
import {
  Button,
  CopyButton,
  Drawer,
  EmptyState,
  Field,
  PageHeader,
  Section,
  Spinner,
  StatusPill,
} from "../components/ui";
import { IconPlug, IconPlus, IconTrash } from "../components/icons";

export function ConnectorsPage() {
  const { principal } = useSession();
  const { success, error } = useToast();
  const queryClient = useQueryClient();
  const [composer, setComposer] = useState(false);
  const [name, setName] = useState("");
  const [region, setRegion] = useState(principal?.tenant.region || "global");
  const [token, setToken] = useState<string | null>(null);

  const connectors = useQuery({
    queryKey: ["connectors"],
    queryFn: () => api<{ connectors: Connector[] }>("/connectors"),
  });

  const create = useMutation({
    mutationFn: () =>
      api<{ connector: Connector; token: string }>("/connectors", {
        method: "POST",
        body: JSON.stringify({ name, region }),
      }),
    onSuccess: (result) => {
      setToken(result.token);
      setName("");
      success("Connector created — copy the token now");
      void queryClient.invalidateQueries({ queryKey: ["connectors"] });
    },
    onError: (reason) => error(errorMessage(reason)),
  });

  const remove = useMutation({
    mutationFn: (id: string) => api(`/connectors/${id}`, { method: "DELETE" }),
    onSuccess: () => {
      success("Connector deleted");
      void queryClient.invalidateQueries({ queryKey: ["connectors"] });
    },
    onError: (reason) => error(errorMessage(reason)),
  });

  const list = connectors.data?.connectors ?? [];

  function submit(event: FormEvent) {
    event.preventDefault();
    setToken(null);
    create.mutate();
  }

  return (
    <>
      <PageHeader
        title="Connectors"
        description="Self-hosted workers that lease jobs from the cloud control plane."
        actions={
          <Button variant="primary" icon={<IconPlus size={16} />} onClick={() => setComposer(true)}>
            New connector
          </Button>
        }
      />

      <div className="banner-note">
        Connectors let you keep browser traffic inside your network. The control plane queues work; the connector
        pulls it, runs it locally and reports back.
      </div>

      <Section title={`Connectors (${list.length})`} description="Registration tokens are shown only once.">
        {connectors.isLoading ? (
          <Spinner label="Loading connectors…" />
        ) : list.length === 0 ? (
          <EmptyState
            icon={<IconPlug size={24} />}
            title="No connectors"
            description="Register one to run work inside your own infrastructure."
            action={
              <Button variant="primary" size="sm" onClick={() => setComposer(true)}>
                Register connector
              </Button>
            }
          />
        ) : (
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>Status</th>
                  <th>Name</th>
                  <th>Region</th>
                  <th>Version</th>
                  <th>Last seen</th>
                  <th>Created</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {list.map((connector) => (
                  <tr key={connector.id}>
                    <td>
                      <StatusPill value={connector.status} />
                    </td>
                    <td>
                      <strong>{connector.name}</strong>
                    </td>
                    <td className="muted small">{connector.region}</td>
                    <td className="muted small">{connector.version ?? "—"}</td>
                    <td className="muted small">{connector.lastSeenAt ? timeAgo(connector.lastSeenAt) : "never"}</td>
                    <td className="muted small">{timeAgo(connector.createdAt)}</td>
                    <td>
                      <Button
                        size="sm"
                        variant="ghost"
                        icon={<IconTrash size={14} />}
                        onClick={() => {
                          if (confirm("Delete this connector? Queued work will be cancelled.")) remove.mutate(connector.id);
                        }}
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
        onClose={() => {
          setComposer(false);
          setToken(null);
        }}
        title="Register connector"
        subtitle="Give it a name and choose the region it serves."
      >
        {token ? (
          <div className="stack">
            <div className="notice ok">
              <strong>Connector token</strong>
              <p className="small">This is shown once. Store it in your connector's environment.</p>
            </div>
            <div className="token-box">
              <code className="mono small">{token}</code>
              <CopyButton value={token} />
            </div>
            <Button variant="primary" onClick={() => setComposer(false)}>
              Done
            </Button>
          </div>
        ) : (
          <form className="stack" onSubmit={submit}>
            <Field label="Name">
              <input value={name} onChange={(event) => setName(event.target.value)} placeholder="eu-browser-1" required />
            </Field>
            <Field label="Region">
              <input value={region} onChange={(event) => setRegion(event.target.value)} required />
            </Field>
            <div className="drawer-form-actions">
              <Button type="button" variant="ghost" onClick={() => setComposer(false)}>
                Cancel
              </Button>
              <Button type="submit" variant="primary" disabled={create.isPending}>
                Register
              </Button>
            </div>
          </form>
        )}
      </Drawer>
    </>
  );
}
