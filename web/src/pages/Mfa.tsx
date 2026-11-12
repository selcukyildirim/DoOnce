import { useState, type FormEvent } from "react";
import { useSession } from "../lib/session";
import { useToast, errorMessage } from "../lib/toast";
import { Button, Field } from "../components/ui";
import { IconShield } from "../components/icons";

export function MfaPage() {
  const { verifyMfa } = useSession();
  const { error } = useToast();
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    try {
      await verifyMfa(code);
    } catch (reason) {
      error(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="auth auth-single">
      <section className="auth-panel">
        <form className="auth-form" onSubmit={submit}>
          <div className="auth-icon">
            <IconShield size={26} />
          </div>
          <div>
            <h2>Two-factor verification</h2>
            <p className="muted">Enter the 6-digit code from your authenticator app to continue.</p>
          </div>
          <Field label="Authentication code">
            <input
              inputMode="numeric"
              autoComplete="one-time-code"
              value={code}
              onChange={(event) => setCode(event.target.value)}
              placeholder="000000"
              className="otp"
              required
            />
          </Field>
          <Button type="submit" variant="primary" disabled={busy}>
            {busy ? "Verifying…" : "Verify"}
          </Button>
        </form>
      </section>
    </div>
  );
}
