import { useEffect, useState } from "react";
import { NavLink, Outlet, useNavigate } from "react-router-dom";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { useSession } from "../lib/session";
import { useToast } from "../lib/toast";
import type { Approval } from "../lib/types";
import {
  IconCalendar,
  IconHome,
  IconKey,
  IconMenu,
  IconMoon,
  IconPlug,
  IconRefresh,
  IconRepo,
  IconRuns,
  IconSettings,
  IconShield,
  IconSpark,
  IconSun,
  IconWorkflow,
  IconX,
} from "./icons";
import { cx } from "./ui";

const NAV = [
  {
    label: "Workspace",
    items: [
      { to: "/", label: "Dashboard", icon: IconHome, end: true },
      { to: "/tasks", label: "Tasks", icon: IconSpark },
      { to: "/workflows", label: "Workflows", icon: IconWorkflow },
      { to: "/runs", label: "Runs", icon: IconRuns },
    ],
  },
  {
    label: "Operations",
    items: [
      { to: "/schedules", label: "Schedules", icon: IconCalendar },
      { to: "/approvals", label: "Approvals", icon: IconShield },
      { to: "/connectors", label: "Connectors", icon: IconPlug },
    ],
  },
  {
    label: "Admin",
    items: [
      { to: "/credentials", label: "Credentials", icon: IconKey },
      { to: "/audit", label: "Audit log", icon: IconRepo },
      { to: "/settings", label: "Settings", icon: IconSettings },
    ],
  },
];

function useTheme() {
  const [theme, setTheme] = useState<"light" | "dark">(
    () => (localStorage.getItem("doonce.theme") as "light" | "dark") || "light",
  );
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    localStorage.setItem("doonce.theme", theme);
  }, [theme]);
  return { theme, toggle: () => setTheme((value) => (value === "light" ? "dark" : "light")) };
}

export function AppShell() {
  const { principal, logout } = useSession();
  const { theme, toggle } = useTheme();
  const [mobileOpen, setMobileOpen] = useState(false);
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const { success, error } = useToast();

  const approvals = useQuery({
    queryKey: ["approvals", "pending"],
    queryFn: () => api<{ approvals: Approval[] }>("/approvals?status=pending"),
    refetchInterval: 15000,
  });

  useEffect(() => {
    setMobileOpen(false);
  }, [navigate]);

  if (!principal) return null;

  const pending = approvals.data?.approvals.length ?? 0;

  async function refreshAll() {
    await queryClient.invalidateQueries();
    success("Console refreshed");
  }

  return (
    <div className="shell">
      <aside className={cx("sidebar", mobileOpen && "open")}>
        <div className="brand">
          <span className="brand-mark">D</span>
          <div className="brand-text">
            <strong>DoOnce</strong>
            <span>{principal.tenant.name}</span>
          </div>
          <button className="icon-btn mobile-only" onClick={() => setMobileOpen(false)} aria-label="Close menu">
            <IconX />
          </button>
        </div>

        <nav className="nav">
          {NAV.map((group) => (
            <div className="nav-group" key={group.label}>
              <span className="nav-label">{group.label}</span>
              {group.items.map((item) => (
                <NavLink
                  key={item.to}
                  to={item.to}
                  end={item.end}
                  className={({ isActive }) => cx("nav-item", isActive && "active")}
                  onClick={() => setMobileOpen(false)}
                >
                  <item.icon size={18} />
                  <span>{item.label}</span>
                  {item.to === "/approvals" && pending > 0 && <span className="nav-count">{pending}</span>}
                </NavLink>
              ))}
            </div>
          ))}
        </nav>

        <div className="sidebar-foot">
          <div className="who">
            <span className="avatar">{principal.tenant.name.slice(0, 1).toUpperCase()}</span>
            <div>
              <strong>{principal.tenant.slug}</strong>
              <span>{principal.role.toLowerCase().replace(/_/g, " ")}</span>
            </div>
          </div>
          <button
            className="btn btn-ghost btn-sm"
            onClick={async () => {
              try {
                await logout();
              } catch (reason) {
                error(reason instanceof Error ? reason.message : String(reason));
              }
            }}
          >
            Log out
          </button>
        </div>
      </aside>

      {mobileOpen && <div className="scrim" onClick={() => setMobileOpen(false)} />}

      <div className="main">
        <header className="topbar">
          <button className="icon-btn mobile-only" onClick={() => setMobileOpen(true)} aria-label="Open menu">
            <IconMenu />
          </button>
          <div className="topbar-context">
            <span className="env-chip">{principal.authMethod === "dev" ? "local dev" : principal.authMethod}</span>
            <span className="muted small">{principal.tenant.region}</span>
          </div>
          <div className="topbar-actions">
            <button className="icon-btn" onClick={refreshAll} aria-label="Refresh data" title="Refresh data">
              <IconRefresh />
            </button>
            <button className="icon-btn" onClick={toggle} aria-label="Toggle theme" title="Toggle theme">
              {theme === "light" ? <IconMoon /> : <IconSun />}
            </button>
            <span className="avatar small-avatar">{principal.role.slice(0, 1)}</span>
          </div>
        </header>
        <main className="content">
          <Outlet />
        </main>
      </div>
    </div>
  );
}
