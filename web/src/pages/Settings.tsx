import { useState, type FormEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { useSession } from "../lib/session";
import { useToast, errorMessage } from "../lib/toast";
import { timeAgo } from "../lib/format";
import type { ApiKey, LlmConfig, Webhook } from "../lib/types";
import {
  Badge,
  Button,
  CopyButton,
  Field,
  PageHeader,
  Section,
  Spinner,
  StatusPill,
} from "../components/ui";
import { IconKey, IconTrash } from "../components/icons";

export function SettingsPage() {
  const { principal } = useSession();
  const { success, error } = useToast();
  const queryClient = useQueryClient();

  const version = useQuery({
    queryKey: ["version"],
    queryFn: () => api<{ version: string; schema: string }>("/version"),
  });

  return (
    <>
      <PageHeader title="Settings" description="Workspace, keys, identity and integrations." />

      <div className="settings-grid">
        <Section title="Workspace" description="Tenant identity and runtime information.">
          <dl className="kv">
            <div>
              <dt>Name</dt>
              <dd>{principal?.tenant.name}</dd>
            </div>
            <div>
              <dt>Slug</dt>
              <dd className="mono">{principal?.tenant.slug}</dd>
            </div>
            <div>
              <dt>Region</dt>
              <dd>{principal?.tenant.region}</dd>
            </div>
            <div>
              <dt>Your role</dt>
              <dd>
                <Badge tone="accent">{principal?.role}</Badge>
              </dd>
            </div>
            <div>
              <dt>Auth method</dt>
              <dd>{principal?.authMethod}</dd>
            </div>
            <div>
              <dt>Version</dt>
              <dd className="mono small">
                {version.data ? `${version.data.version} · schema ${version.data.schema}` : "—"}
              </dd>
            </div>
          </dl>
        </Section>

        <LlmProviders success={success} error={error} />
        <SettingsValues onSaved={() => queryClient.invalidateQueries({ queryKey: ["settings"] })} />
        <ApiKeys success={success} error={error} />
        <Webhooks success={success} error={error} />
      </div>
    </>
  );
}

function SettingsValues({ onSaved }: { onSaved: () => void }) {
  const { success, error } = useToast();
  const [key, setKey] = useState("");
  const [value, setValue] = useState("");

  const settings = useQuery({
    queryKey: ["settings"],
    queryFn: () => api<{ settings: Record<string, string> }>("/settings"),
  });

  const save = useMutation({
    mutationFn: () => api("/settings", { method: "PUT", body: JSON.stringify({ key, value }) }),
    onSuccess: () => {
      success("Setting saved");
      setKey("");
      setValue("");
      onSaved();
    },
    onError: (reason) => error(errorMessage(reason)),
  });

  const entries = Object.entries(settings.data?.settings ?? {});

  return (
    <Section title="Workspace settings" description="Key/value configuration shared by server and workers.">
      <div className="stack">
        {settings.isLoading ? (
          <Spinner />
        ) : entries.length === 0 ? (
          <p className="muted small">No overrides set.</p>
        ) : (
          <dl className="kv">
            {entries.map(([name, entry]) => (
              <div key={name}>
                <dt className="mono small">{name}</dt>
                <dd className="mono small">{entry}</dd>
              </div>
            ))}
          </dl>
        )}
        <form
          className="inline-form"
          onSubmit={(event: FormEvent) => {
            event.preventDefault();
            save.mutate();
          }}
        >
          <input value={key} onChange={(event) => setKey(event.target.value)} placeholder="key" required />
          <input value={value} onChange={(event) => setValue(event.target.value)} placeholder="value" required />
          <Button type="submit" variant="secondary" disabled={save.isPending}>
            Save
          </Button>
        </form>
      </div>
    </Section>
  );
}

function ApiKeys({ success, error }: { success: (m: string) => void; error: (m: string) => void }) {
  const queryClient = useQueryClient();
  const [name, setName] = useState("");
  const [token, setToken] = useState<string | null>(null);

  const keys = useQuery({
    queryKey: ["api-keys"],
    queryFn: () => api<{ apiKeys: ApiKey[] }>("/api-keys"),
  });

  const create = useMutation({
    mutationFn: () => api<{ token: string }>("/api-keys", { method: "POST", body: JSON.stringify({ name: name || "console" }) }),
    onSuccess: (result) => {
      setToken(result.token);
      setName("");
      success("API key created — copy it now");
      void queryClient.invalidateQueries({ queryKey: ["api-keys"] });
    },
    onError: (reason) => error(errorMessage(reason)),
  });

  const revoke = useMutation({
    mutationFn: (id: string) => api(`/api-keys/${id}`, { method: "DELETE" }),
    onSuccess: () => {
      success("API key revoked");
      void queryClient.invalidateQueries({ queryKey: ["api-keys"] });
    },
    onError: (reason) => error(errorMessage(reason)),
  });

  const list = keys.data?.apiKeys ?? [];

  return (
    <Section title="API keys" description="Programmatic access for SDKs, MCP and connectors." actions={<Badge tone="muted">{list.length}</Badge>}>
      <div className="stack">
        {token && (
          <div className="token-box">
            <code className="mono small">{token}</code>
            <CopyButton value={token} />
          </div>
        )}
        <div className="row-list">
          {list.map((key) => (
            <div className="row-item" key={key.id}>
              <div className="row-main">
                <IconKey size={16} />
                <strong>{key.name}</strong>
                <span className="mono small muted">{key.prefix}…</span>
                {key.revokedAt && <StatusPill value="disabled" />}
              </div>
              <div className="row">
                <span className="muted small">{key.lastUsedAt ? timeAgo(key.lastUsedAt) : "never used"}</span>
                {!key.revokedAt && (
                  <Button
                    size="sm"
                    variant="ghost"
                    icon={<IconTrash size={13} />}
                    onClick={() => {
                      if (confirm("Revoke this API key?")) revoke.mutate(key.id);
                    }}
                  >
                    Revoke
                  </Button>
                )}
              </div>
            </div>
          ))}
          {list.length === 0 && <p className="muted small">No API keys yet.</p>}
        </div>
        <form
          className="inline-form"
          onSubmit={(event: FormEvent) => {
            event.preventDefault();
            create.mutate();
          }}
        >
          <input value={name} onChange={(event) => setName(event.target.value)} placeholder="Key name" />
          <Button type="submit" variant="secondary" disabled={create.isPending}>
            Create key
          </Button>
        </form>
      </div>
    </Section>
  );
}

function Webhooks({ success, error }: { success: (m: string) => void; error: (m: string) => void }) {
  const queryClient = useQueryClient();
  const [url, setUrl] = useState("");
  const [events, setEvents] = useState("run.completed, run.failed");
  const [secret, setSecret] = useState<string | null>(null);

  const webhooks = useQuery({
    queryKey: ["webhooks"],
    queryFn: () => api<{ webhooks: Webhook[] }>("/webhooks"),
  });

  const create = useMutation({
    mutationFn: () =>
      api<{ secret?: string | null }>("/webhooks", {
        method: "POST",
        body: JSON.stringify({
          url,
          events: events.split(",").map((item) => item.trim()).filter(Boolean),
        }),
      }),
    onSuccess: (result) => {
      setSecret(result.secret ?? null);
      setUrl("");
      success("Webhook created");
      void queryClient.invalidateQueries({ queryKey: ["webhooks"] });
    },
    onError: (reason) => error(errorMessage(reason)),
  });

  const remove = useMutation({
    mutationFn: (id: string) => api(`/webhooks/${id}`, { method: "DELETE" }),
    onSuccess: () => {
      success("Webhook deleted");
      void queryClient.invalidateQueries({ queryKey: ["webhooks"] });
    },
    onError: (reason) => error(errorMessage(reason)),
  });

  const list = webhooks.data?.webhooks ?? [];

  return (
    <Section title="Webhooks" description="Outbound notifications with signed payloads." actions={<Badge tone="muted">{list.length}</Badge>}>
      <div className="stack">
        {secret && (
          <div className="token-box">
            <code className="mono small">{secret}</code>
            <CopyButton value={secret} />
          </div>
        )}
        <div className="row-list">
          {list.map((webhook) => (
            <div className="row-item" key={webhook.id}>
              <div className="row-main">
                <span className="mono small">{webhook.url}</span>
                <span className="muted small">{webhook.events.join(", ")}</span>
              </div>
              <div className="row">
                <StatusPill value={webhook.enabled ? "active" : "disabled"} />
                <Button
                  size="sm"
                  variant="ghost"
                  icon={<IconTrash size={13} />}
                  onClick={() => {
                    if (confirm("Delete this webhook?")) remove.mutate(webhook.id);
                  }}
                >
                  Remove
                </Button>
              </div>
            </div>
          ))}
          {list.length === 0 && <p className="muted small">No webhooks yet.</p>}
        </div>
        <form
          className="stack"
          onSubmit={(event: FormEvent) => {
            event.preventDefault();
            create.mutate();
          }}
        >
          <Field label="Endpoint URL">
            <input value={url} onChange={(event) => setUrl(event.target.value)} placeholder="https://…" required />
          </Field>
          <Field label="Events" hint="Comma-separated, e.g. run.completed, run.failed">
            <input value={events} onChange={(event) => setEvents(event.target.value)} />
          </Field>
          <Button type="submit" variant="secondary" disabled={create.isPending}>
            Add webhook
          </Button>
        </form>
      </div>
    </Section>
  );
}


function LlmProviders({ success, error }: { success: (m: string) => void; error: (m: string) => void }) {
  const queryClient = useQueryClient();
  const [provider, setProvider] = useState("openai-compatible");
  const [model, setModel] = useState("deepseek-chat");
  const [baseUrl, setBaseUrl] = useState("https://api.deepseek.com");
  const [apiKey, setApiKey] = useState("");
  const [isDefault, setIsDefault] = useState(true);
  const [testResult, setTestResult] = useState<string | null>(null);

  const configs = useQuery({
    queryKey: ["llm-configs"],
    queryFn: () => api<{ configs: LlmConfig[] }>("/llm-configs"),
  });

  const create = useMutation({
    mutationFn: () =>
      api("/llm-configs", {
        method: "POST",
        body: JSON.stringify({
          provider,
          model,
          base_url: baseUrl || undefined,
          api_key: apiKey || undefined,
          is_default: isDefault,
          capabilities: { vision: false, json_mode: true },
        }),
      }),
    onSuccess: () => {
      success("AI provider saved");
      setApiKey("");
      setTestResult(null);
      void queryClient.invalidateQueries({ queryKey: ["llm-configs"] });
    },
    onError: (reason) => error(errorMessage(reason)),
  });

  const remove = useMutation({
    mutationFn: (id: string) => api(`/llm-configs/${id}`, { method: "DELETE" }),
    onSuccess: () => {
      success("AI provider removed");
      void queryClient.invalidateQueries({ queryKey: ["llm-configs"] });
    },
    onError: (reason) => error(errorMessage(reason)),
  });

  const setDef = useMutation({
    mutationFn: (id: string) => api(`/llm-configs/${id}/default`, { method: "POST" }),
    onSuccess: () => {
      success("Default provider updated");
      void queryClient.invalidateQueries({ queryKey: ["llm-configs"] });
    },
    onError: (reason) => error(errorMessage(reason)),
  });

  const test = useMutation({
    mutationFn: () =>
      api<{ ok: boolean; reply?: string; error?: string }>("/llm-configs/test", {
        method: "POST",
        body: JSON.stringify({ provider, model, base_url: baseUrl || undefined, api_key: apiKey || undefined }),
      }),
    onSuccess: (result) => {
      setTestResult(result.ok ? `OK: ${result.reply ?? "connected"}` : `FAIL: ${result.error ?? "unknown"}`);
    },
    onError: (reason) => setTestResult(`FAIL: ${errorMessage(reason)}`),
  });

  const list = configs.data?.configs ?? [];

  return (
    <Section title="AI providers" description="LLM providers used by AI tasks and browserTask blocks. Keys are encrypted at rest.">
      <div className="stack">
        <div className="row-list">
          {list.map((config) => (
            <div className="row-item" key={config.id}>
              <div className="row-main">
                <strong>{config.provider}</strong>
                <span className="mono small muted">{config.model}</span>
                {config.baseUrl && <span className="muted small">{config.baseUrl}</span>}
                {config.isDefault && <Badge tone="accent">default</Badge>}
                {config.hasApiKey && <Badge tone="ok">key set</Badge>}
              </div>
              <div className="row">
                {!config.isDefault && (
                  <Button size="sm" variant="ghost" onClick={() => setDef.mutate(config.id)}>
                    Make default
                  </Button>
                )}
                <Button size="sm" variant="ghost" icon={<IconTrash size={13} />} onClick={() => { if (confirm("Delete this provider?")) remove.mutate(config.id); }}>
                  Remove
                </Button>
              </div>
            </div>
          ))}
          {list.length === 0 && <p className="muted small">No providers configured. Add one below or set DOONCE_LLM_* env vars.</p>}
        </div>

        <div className="stack" style={{ borderTop: "1px solid var(--border)", paddingTop: "1rem" }}>
          <div className="grid2">
            <Field label="Provider">
              <select value={provider} onChange={(e) => setProvider(e.target.value)}>
                <option value="openai">openai</option>
                <option value="anthropic">anthropic</option>
                <option value="gemini">gemini</option>
                <option value="ollama">ollama</option>
                <option value="openai-compatible">openai-compatible</option>
              </select>
            </Field>
            <Field label="Model">
              <input value={model} onChange={(e) => setModel(e.target.value)} placeholder="deepseek-chat" />
            </Field>
          </div>
          <Field label="Base URL (optional)">
            <input value={baseUrl} onChange={(e) => setBaseUrl(e.target.value)} placeholder="https://api.deepseek.com" />
          </Field>
          <Field label="API key" hint="Stored encrypted. Leave blank to keep existing.">
            <input type="password" value={apiKey} onChange={(e) => setApiKey(e.target.value)} autoComplete="new-password" />
          </Field>
          <div className="row">
            <label className="row" style={{ gap: "0.4rem" }}>
              <input type="checkbox" checked={isDefault} onChange={(e) => setIsDefault(e.target.checked)} style={{ width: "auto" }} />
              <span className="small">Set as default</span>
            </label>
          </div>
          {testResult && (
            <div className={testResult.startsWith("OK") ? "notice ok" : "notice bad"}>
              <span className="small">{testResult}</span>
            </div>
          )}
          <div className="row">
            <Button variant="secondary" onClick={() => test.mutate()} disabled={test.isPending}>
              {test.isPending ? "Testing…" : "Test connection"}
            </Button>
            <Button variant="primary" onClick={() => create.mutate()} disabled={create.isPending || !provider.trim() || !model.trim()}>
              {create.isPending ? "Saving…" : "Save provider"}
            </Button>
          </div>
        </div>
      </div>
    </Section>
  );
}
