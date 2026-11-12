import { useEffect, useState, type ButtonHTMLAttributes, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { IconCopy, IconX } from "./icons";

export function cx(...values: Array<string | false | null | undefined>) {
  return values.filter(Boolean).join(" ");
}

type ButtonVariant = "primary" | "secondary" | "ghost" | "danger" | "quiet";
type ButtonSize = "sm" | "md";

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  icon?: ReactNode;
}

export function Button({ variant = "secondary", size = "md", icon, className, children, ...props }: ButtonProps) {
  return (
    <button className={cx("btn", `btn-${variant}`, `btn-${size}`, className)} {...props}>
      {icon && <span className="btn-icon">{icon}</span>}
      {children}
    </button>
  );
}

export function Card({ className, children }: { className?: string; children: ReactNode }) {
  return <div className={cx("card", className)}>{children}</div>;
}

export function Section({
  title,
  description,
  actions,
  children,
  className,
}: {
  title?: ReactNode;
  description?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={cx("card", className)}>
      {(title || actions) && (
        <div className="card-head">
          <div>
            {title && <h3>{title}</h3>}
            {description && <p className="muted small">{description}</p>}
          </div>
          {actions && <div className="row">{actions}</div>}
        </div>
      )}
      {children}
    </section>
  );
}

const STATUS_TONES: Record<string, string> = {
  completed: "ok",
  succeeded: "ok",
  success: "ok",
  running: "info",
  queued: "warn",
  pending: "warn",
  awaiting_approval: "warn",
  awaiting_human: "warn",
  failed: "bad",
  error: "bad",
  cancelled: "muted",
  disabled: "muted",
  active: "ok",
  online: "ok",
  offline: "muted",
  approved: "ok",
  denied: "bad",
};

export function StatusPill({ value }: { value: string }) {
  const tone = STATUS_TONES[value] ?? "muted";
  const label = value.replace(/_/g, " ");
  return (
    <span className={cx("pill", `pill-${tone}`)}>
      <span className="pill-dot" />
      {label}
    </span>
  );
}

export function Badge({ children, tone = "muted" }: { children: ReactNode; tone?: string }) {
  return <span className={cx("badge", `badge-${tone}`)}>{children}</span>;
}

export function Field({
  label,
  hint,
  children,
  className,
}: {
  label: string;
  hint?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <label className={cx("field", className)}>
      <span className="field-label">{label}</span>
      {children}
      {hint && <span className="field-hint">{hint}</span>}
    </label>
  );
}

export function EmptyState({
  icon,
  title,
  description,
  action,
}: {
  icon?: ReactNode;
  title: string;
  description?: string;
  action?: ReactNode;
}) {
  return (
    <div className="empty">
      {icon && <div className="empty-icon">{icon}</div>}
      <strong>{title}</strong>
      {description && <p className="muted small">{description}</p>}
      {action}
    </div>
  );
}

export function Skeleton({ className }: { className?: string }) {
  return <div className={cx("skeleton", className)} />;
}

export function PageHeader({
  title,
  description,
  actions,
}: {
  title: string;
  description?: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <header className="page-head">
      <div>
        <h1>{title}</h1>
        {description && <p className="muted">{description}</p>}
      </div>
      {actions && <div className="row">{actions}</div>}
    </header>
  );
}

export function StatCard({
  label,
  value,
  hint,
  tone,
  icon,
}: {
  label: string;
  value: ReactNode;
  hint?: ReactNode;
  tone?: "accent" | "ok" | "bad" | "warn";
  icon?: ReactNode;
}) {
  return (
    <div className={cx("stat", tone && `stat-${tone}`)}>
      <div className="stat-top">
        <span className="stat-label">{label}</span>
        {icon && <span className="stat-icon">{icon}</span>}
      </div>
      <strong className="stat-value">{value}</strong>
      {hint && <span className="stat-hint">{hint}</span>}
    </div>
  );
}

export function Spinner({ label }: { label?: string }) {
  return (
    <div className="spinner-row">
      <span className="spinner" />
      {label && <span className="muted small">{label}</span>}
    </div>
  );
}

export function Drawer({
  open,
  onClose,
  title,
  subtitle,
  children,
  footer,
}: {
  open: boolean;
  onClose: () => void;
  title: ReactNode;
  subtitle?: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
}) {
  useEffect(() => {
    if (!open) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    document.body.classList.add("no-scroll");
    return () => {
      document.removeEventListener("keydown", onKey);
      document.body.classList.remove("no-scroll");
    };
  }, [open, onClose]);

  if (!open) return null;
  return createPortal(
    <div className="drawer-layer">
      <div className="drawer-scrim" onClick={onClose} />
      <aside className="drawer" role="dialog" aria-modal="true">
        <header className="drawer-head">
          <div>
            <h2>{title}</h2>
            {subtitle && <div className="muted small">{subtitle}</div>}
          </div>
          <button className="icon-btn" onClick={onClose} aria-label="Close">
            <IconX />
          </button>
        </header>
        <div className="drawer-body">{children}</div>
        {footer && <footer className="drawer-foot">{footer}</footer>}
      </aside>
    </div>,
    document.body,
  );
}

export function CopyButton({ value, label = "Copy" }: { value: string; label?: string }) {
  return (
    <Button
      type="button"
      variant="quiet"
      size="sm"
      icon={<IconCopy size={15} />}
      onClick={() => {
        void navigator.clipboard?.writeText(value);
      }}
    >
      {label}
    </Button>
  );
}

export function Toggle({
  checked,
  onChange,
  label,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      className={cx("toggle", checked && "on")}
      onClick={() => onChange(!checked)}
    >
      <span />
    </button>
  );
}

export function CodeBlock({ children, className }: { children: ReactNode; className?: string }) {
  return <pre className={cx("code", className)}>{children}</pre>;
}

export function ResultView({ value }: { value: unknown }) {
  const [raw, setRaw] = useState(false);
  if (value == null) return <p className="muted small">No result.</p>;
  return (
    <div className="result-view">
      <div className="row" style={{ justifyContent: "flex-end", marginBottom: "0.5rem" }}>
        <button type="button" className={raw ? "tab active" : "tab"} onClick={() => setRaw((v) => !v)}>
          {raw ? "Pretty" : "Raw JSON"}
        </button>
      </div>
      {raw ? (
        <CodeBlock>{JSON.stringify(value, null, 2)}</CodeBlock>
      ) : (
        <RenderValue value={value} />
      )}
    </div>
  );
}

function RenderValue({ value }: { value: unknown }) {
  if (value == null) return <span className="muted">—</span>;
  if (typeof value === "string") return <p>{value}</p>;
  if (typeof value === "number" || typeof value === "boolean") return <p className="mono">{String(value)}</p>;
  if (Array.isArray(value)) {
    if (value.length === 0) return <p className="muted small">Empty list.</p>;
    if (value.every((item) => item && typeof item === "object" && !Array.isArray(item))) {
      const keys = Array.from(new Set(value.flatMap((item) => Object.keys(item as Record<string, unknown>))));
      return (
        <div className="table-wrap">
          <table>
            <thead>
              <tr>
                {keys.map((key) => (
                  <th key={key}>{key}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {value.map((item, index) => (
                <tr key={index}>
                  {keys.map((key) => {
                    const cell = (item as Record<string, unknown>)[key];
                    return (
                      <td key={key} className="clip">
                        {cell == null ? "—" : typeof cell === "object" ? JSON.stringify(cell) : String(cell)}
                      </td>
                    );
                  })}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    }
    return (
      <div className="stack">
        {value.map((item, index) => (
          <div className="result-item" key={index}>
            <RenderValue value={item} />
          </div>
        ))}
      </div>
    );
  }
  if (typeof value === "object") {
    const entries = Object.entries(value as Record<string, unknown>);
    // If there's a single array field + scalar summary, render specially
    const arrayField = entries.find(([, v]) => Array.isArray(v));
    const scalarFields = entries.filter(([, v]) => !Array.isArray(v));
    return (
      <div className="stack">
        {scalarFields.map(([key, val]) => (
          <div className="row-between" key={key}>
            <span className="field-label">{key}</span>
            <span className="small">{val == null ? "—" : String(val)}</span>
          </div>
        ))}
        {arrayField && (
          <div>
            <span className="field-label">{arrayField[0]}</span>
            <RenderValue value={arrayField[1]} />
          </div>
        )}
        {entries.filter(([k]) => k !== arrayField?.[0] && !scalarFields.some(([sk]) => sk === k)).map(([key, val]) => (
          <div key={key}>
            <span className="field-label">{key}</span>
            <RenderValue value={val} />
          </div>
        ))}
      </div>
    );
  }
  return <p>{String(value)}</p>;
}
