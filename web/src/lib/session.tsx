import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { api, clearApiKey, hasApiKey, setApiKey } from "./api";
import type { Principal, ProviderInfo } from "./types";

interface SessionValue {
  principal: Principal | null;
  providers: ProviderInfo[];
  checking: boolean;
  loginWithKey: (key: string) => Promise<void>;
  logout: () => Promise<void>;
  verifyMfa: (code: string) => Promise<void>;
  reload: () => Promise<void>;
}

const SessionContext = createContext<SessionValue | null>(null);

export function SessionProvider({ children }: { children: ReactNode }) {
  const [principal, setPrincipal] = useState<Principal | null>(null);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [checking, setChecking] = useState(true);

  const loadPrincipal = useCallback(async () => {
    const me = await api<Principal>("/auth/me");
    setPrincipal(me);
    return me;
  }, []);

  const reload = useCallback(async () => {
    try {
      await loadPrincipal();
    } catch {
      setPrincipal(null);
    }
  }, [loadPrincipal]);

  useEffect(() => {
    api<Principal>("/auth/me")
      .then(setPrincipal)
      .catch(() => {
        if (hasApiKey()) clearApiKey();
        setPrincipal(null);
      })
      .finally(() => setChecking(false));
    api<{ providers: ProviderInfo[] }>("/auth/providers?tenant=default")
      .then((data) => setProviders(data.providers))
      .catch(() => setProviders([]));
  }, []);

  const loginWithKey = useCallback(
    async (key: string) => {
      setApiKey(key);
      setChecking(true);
      try {
        await loadPrincipal();
      } catch (reason) {
        clearApiKey();
        throw reason;
      } finally {
        setChecking(false);
      }
    },
    [loadPrincipal],
  );

  const logout = useCallback(async () => {
    try {
      await api("/auth/logout", { method: "POST" });
    } catch {
      /* the session may already be gone */
    }
    clearApiKey();
    setPrincipal(null);
  }, []);

  const verifyMfa = useCallback(
    async (code: string) => {
      await api("/auth/mfa/verify", { method: "POST", body: JSON.stringify({ code }) });
      await loadPrincipal();
    },
    [loadPrincipal],
  );

  const value = useMemo<SessionValue>(
    () => ({ principal, providers, checking, loginWithKey, logout, verifyMfa, reload }),
    [principal, providers, checking, loginWithKey, logout, verifyMfa, reload],
  );

  return <SessionContext.Provider value={value}>{children}</SessionContext.Provider>;
}

export function useSession() {
  const value = useContext(SessionContext);
  if (!value) throw new Error("useSession must be used inside SessionProvider");
  return value;
}
