import { useState, type FormEvent } from "react";
import { useSession } from "../lib/session";
import { useToast, errorMessage } from "../lib/toast";
import { Button, Field } from "../components/ui";
import { IconSpark, IconShield, IconZap } from "../components/icons";

export function LoginPage() {
  const { providers, loginWithKey } = useSession();
  const { error } = useToast();
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    try {
      await loginWithKey(key);
    } catch (reason) {
      error(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="auth">
      <section className="auth-story">
        <div className="auth-brand">
          <span className="brand-mark large">D</span>
          <span>DoOnce</span>
        </div>
        <h1>Browser work, done once and done right.</h1>
        <p>
          Describe the job. DoOnce runs it in a real browser, keeps the receipts, and hands the result back as clean
          data.
        </p>
        <ul className="auth-points">
          <li>
            <IconSpark size={18} />
            <div>
              <strong>Prompt to outcome</strong>
              <span>AI agents and deterministic workflows in one place.</span>
            </div>
          </li>
          <li>
            <IconShield size={18} />
            <div>
              <strong>Enterprise controls</strong>
              <span>RBAC, SSO, SCIM, audit trails and tenant isolation.</span>
            </div>
          </li>
          <li>
            <IconZap size={18} />
            <div>
              <strong>Observable runs</strong>
              <span>Every step, screenshot and output is inspectable.</span>
            </div>
          </li>
        </ul>
        <div className="auth-glow" />
      </section>

      <section className="auth-panel">
        <form className="auth-form" onSubmit={submit}>
          <div>
            <h2>Sign in to the console</h2>
            <p className="muted">Use your identity provider or an API key issued by your workspace.</p>
          </div>

          {providers.length > 0 && (
            <div className="stack">
              {providers.map((provider) => (
                <Button
                  key={provider.id}
                  type="button"
                  variant="secondary"
                  onClick={() => {
                    window.location.href = "/api/v1/auth/oidc/start?tenant=default";
                  }}
                >
                  Continue with {provider.name}
                </Button>
              ))}
              <div className="divider">
                <span>or use an API key</span>
              </div>
            </div>
          )}

          <Field label="API key" hint="Stored only in this browser. Never sent anywhere else.">
            <input
              type="password"
              autoComplete="off"
              value={key}
              onChange={(event) => setKey(event.target.value)}
              placeholder="dsk_..."
              required
            />
          </Field>

          <Button type="submit" variant="primary" disabled={busy}>
            {busy ? "Connecting…" : "Enter console"}
          </Button>
        </form>
      </section>
    </div>
  );
}
