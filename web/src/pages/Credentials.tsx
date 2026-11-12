import { useState, type FormEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { useToast, errorMessage } from "../lib/toast";
import { timeAgo } from "../lib/format";
import type { Credential } from "../lib/types";
import {
  Badge,
  Button,
  Drawer,
  EmptyState,
  Field,
  PageHeader,
  Section,
  Spinner,
} from "../components/ui";
import { IconKey, IconPlus, IconTrash } from "../components/icons";

const KINDS = ["username_password", "api_key", "totp", "oauth"];

export function CredentialsPage() {
  const { success, error } = useToast();
  const queryClient = useQueryClient();
  const [composer, setComposer] = useState(false);
  const [name, setName] = useState("");
  const [kind, setKind] = useState(KINDS[0]);
  const [provider, setProvider] = useState("local");
  const [externalRef, setExternalRef] = useState("");
  const [username, setUsername] = useState("");
  const [secret, setSecret] = useState("");

  const credentials = useQuery({
    queryKey: ["credentials"],
    queryFn: () => api<{ credentials: Credential[] }>("/credentials"),
  });

  const create = useMutation({
    mutationFn: () =>
      api("/credentials", {
        method: "POST",
        body: JSON.stringify({
          name,
          kind,
          provider,
          externalRef: provider === "local" ? undefined : externalRef || undefined,
          username: username || undefined,
          secret: provider === "local" ? secret : undefined,
        }),
      }),
    onSuccess: () => {
      success("Credential encrypted and stored");
      setComposer(false);
      setName("");
      setExternalRef("");
      setUsername("");
      setSecret("");
      void queryClient.invalidateQueries({ queryKey: ["credentials"] });
    },
    onError: (reason) => error(errorMessage(reason)),
  });

  const remove = useMutation({
    mutationFn: (id: string) => api(`/credentials/${id}`, { method: "DELETE" }),
    onSuccess: () => {
      success("Credential deleted");
      void queryClient.invalidateQueries({ queryKey: ["credentials"] });
    },
    onError: (reason) => error(errorMessage(reason)),
  });

  const list = credentials.data?.credentials ?? [];

  function submit(event: FormEvent) {
    event.preventDefault();
    create.mutate();
  }

  return (
    <>
      <PageHeader
        title="Credentials"
        description="Encrypted secrets and external vault references used by login blocks."
        actions={
          <Button variant="primary" icon={<IconPlus size={16} />} onClick={() => setComposer(true)}>
            Add credential
          </Button>
        }
      />

      <div className="banner-note">
        Secrets are encrypted at rest with the workspace master key. Values are never returned by the API after they are
        written.
      </div>

      <Section title={`Stored credentials (${list.length})`}>
        {credentials.isLoading ? (
          <Spinner label="Loading credentials…" />
        ) : list.length === 0 ? (
          <EmptyState
            icon={<IconKey size={24} />}
            title="No credentials"
            description="Add a login or an API secret to use in workflows."
            action={
              <Button variant="primary" size="sm" onClick={() => setComposer(true)}>
                Add credential
              </Button>
            }
          />
        ) : (
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>Name</th>
                  <th>Kind</th>
                  <th>Provider</th>
                  <th>Username</th>
                  <th>Created</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {list.map((credential) => (
                  <tr key={credential.id}>
                    <td>
                      <strong>{credential.name}</strong>
                    </td>
                    <td>
                      <Badge tone="muted">{credential.kind.replace(/_/g, " ")}</Badge>
                    </td>
                    <td className="muted small">
                      {credential.provider}
                      {credential.externalRef ? ` · ${credential.externalRef}` : ""}
                    </td>
                    <td className="muted small">{credential.username ?? "—"}</td>
                    <td className="muted small">{timeAgo(credential.createdAt)}</td>
                    <td>
                      <Button
                        size="sm"
                        variant="ghost"
                        icon={<IconTrash size={14} />}
                        onClick={() => {
                          if (confirm("Delete this credential?")) remove.mutate(credential.id);
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
        onClose={() => setComposer(false)}
        title="Add credential"
        subtitle="Stored encrypted; plaintext is never returned."
      >
        <form className="stack" onSubmit={submit}>
          <Field label="Name">
            <input value={name} onChange={(event) => setName(event.target.value)} placeholder="Boyner login" required />
          </Field>
          <div className="grid2">
            <Field label="Kind">
              <select value={kind} onChange={(event) => setKind(event.target.value)}>
                {KINDS.map((item) => (
                  <option key={item} value={item}>
                    {item.replace(/_/g, " ")}
                  </option>
                ))}
              </select>
            </Field>
            <Field label="Provider">
              <select value={provider} onChange={(event) => setProvider(event.target.value)}>
                <option value="local">local vault</option>
                <option value="external">external vault</option>
              </select>
            </Field>
          </div>
          {provider === "local" ? (
            <>
              <Field label="Username (optional)">
                <input value={username} onChange={(event) => setUsername(event.target.value)} autoComplete="off" />
              </Field>
              <Field label="Secret">
                <input
                  type="password"
                  value={secret}
                  onChange={(event) => setSecret(event.target.value)}
                  autoComplete="new-password"
                  required
                />
              </Field>
            </>
          ) : (
            <Field label="External reference" hint="The key name in your vault provider.">
              <input value={externalRef} onChange={(event) => setExternalRef(event.target.value)} required />
            </Field>
          )}
          <div className="drawer-form-actions">
            <Button type="button" variant="ghost" onClick={() => setComposer(false)}>
              Cancel
            </Button>
            <Button type="submit" variant="primary" disabled={create.isPending}>
              Save credential
            </Button>
          </div>
        </form>
      </Drawer>
    </>
  );
}
