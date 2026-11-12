import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { BrowserRouter, Route, Routes } from "react-router-dom";
import { SessionProvider, useSession } from "./lib/session";
import { ToastProvider } from "./lib/toast";
import { AppShell } from "./components/AppShell";
import { LoginPage } from "./pages/Login";
import { MfaPage } from "./pages/Mfa";
import { DashboardPage } from "./pages/Dashboard";
import { TasksPage } from "./pages/Tasks";
import { WorkflowsPage } from "./pages/Workflows";
import { RunsPage } from "./pages/Runs";
import { SchedulesPage } from "./pages/Schedules";
import { ApprovalsPage } from "./pages/Approvals";
import { ConnectorsPage } from "./pages/Connectors";
import { CredentialsPage } from "./pages/Credentials";
import { AuditPage } from "./pages/Audit";
import { SettingsPage } from "./pages/Settings";
import { Button, EmptyState } from "./components/ui";
import { IconAlert } from "./components/icons";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 1,
      refetchOnWindowFocus: false,
      staleTime: 4000,
    },
  },
});

function Splash() {
  return (
    <div className="splash">
      <span className="brand-mark large">D</span>
      <span className="splash-text">DoOnce</span>
      <span className="spinner" />
    </div>
  );
}

function NotFound() {
  return (
    <EmptyState
      icon={<IconAlert size={26} />}
      title="Page not found"
      description="This route does not exist in the console."
      action={
        <a href="/">
          <Button variant="secondary">Back to dashboard</Button>
        </a>
      }
    />
  );
}

function Gate() {
  const { principal, checking } = useSession();

  if (checking) return <Splash />;
  if (!principal) return <LoginPage />;
  if (principal.mfaRequired) return <MfaPage />;

  return (
    <Routes>
      <Route element={<AppShell />}>
        <Route index element={<DashboardPage />} />
        <Route path="tasks" element={<TasksPage />} />
        <Route path="tasks/:id" element={<TasksPage />} />
        <Route path="workflows" element={<WorkflowsPage />} />
        <Route path="runs" element={<RunsPage />} />
        <Route path="runs/:id" element={<RunsPage />} />
        <Route path="schedules" element={<SchedulesPage />} />
        <Route path="approvals" element={<ApprovalsPage />} />
        <Route path="connectors" element={<ConnectorsPage />} />
        <Route path="credentials" element={<CredentialsPage />} />
        <Route path="audit" element={<AuditPage />} />
        <Route path="settings" element={<SettingsPage />} />
        <Route path="*" element={<NotFound />} />
      </Route>
    </Routes>
  );
}

export function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <SessionProvider>
          <BrowserRouter>
            <Gate />
          </BrowserRouter>
        </SessionProvider>
      </ToastProvider>
    </QueryClientProvider>
  );
}
