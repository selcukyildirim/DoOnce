import { HOST_NAME, type HostEvent } from "../shared/messages";

type Pending = {
  resolve: (v: unknown) => void;
  reject: (e: Error) => void;
};

let port: chrome.runtime.Port | null = null;
let driverHandler: ((op: string, params: Record<string, unknown>) => Promise<unknown>) | null = null;

export function setDriverHandler(
  fn: (op: string, params: Record<string, unknown>) => Promise<unknown>,
) {
  driverHandler = fn;
}
let nextId = 1;
const pending = new Map<number, Pending>();
const driverPending = new Map<string, Pending>();
const listeners = new Set<(ev: HostEvent | Record<string, unknown>) => void>();

function onPortMessage(msg: Record<string, unknown>) {
  if (typeof msg.driver === "string" && typeof msg.id === "string" && driverHandler) {
    const reqId = msg.id;
    const params = (msg.params as Record<string, unknown>) ?? {};
    driverHandler(msg.driver, params)
      .then((result) => port?.postMessage({ id: reqId, result }))
      .catch((e: Error & { transient?: boolean }) =>
        port?.postMessage({
          id: reqId,
          error: { message: e.message ?? "driver error", transient: !!e.transient },
        }),
      );
    return;
  }
  const id = msg.id;
  if (typeof id === "string" && driverPending.has(id)) {
    const p = driverPending.get(id)!;
    driverPending.delete(id);
    if (msg.error && typeof msg.error === "object") {
      const err = msg.error as { message?: string; transient?: boolean };
      p.reject(Object.assign(new Error(err.message ?? "browser error"), { transient: !!err.transient }));
    } else {
      p.resolve(msg.result ?? null);
    }
    return;
  }
  if (typeof id === "number" && pending.has(id)) {
    const p = pending.get(id)!;
    pending.delete(id);
    if (msg.error && typeof msg.error === "object") {
      p.reject(new Error((msg.error as { message?: string }).message ?? "host error"));
    } else {
      p.resolve(msg.result ?? null);
    }
    return;
  }
  if (msg.event) {
    listeners.forEach((fn) => fn(msg as HostEvent));
  }
}

export function onHostEvent(fn: (ev: HostEvent | Record<string, unknown>) => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

export function hostConnected(): boolean {
  return port !== null;
}

export function connectHost(): Promise<{ version: string; dataDir: string }> {
  if (port) {
    return hostCall("hello", {});
  }
  return new Promise((resolve, reject) => {
    try {
      port = chrome.runtime.connectNative(HOST_NAME);
    } catch (e) {
      reject(e instanceof Error ? e : new Error(String(e)));
      return;
    }
    port.onMessage.addListener(onPortMessage);
    port.onDisconnect.addListener(() => {
      port = null;
      pending.forEach((p) => p.reject(new Error("DoOnce host disconnected. Run: doonce setup")));
      pending.clear();
      driverPending.forEach((p) => p.reject(new Error("host disconnected")));
      driverPending.clear();
    });
    hostCall<{ version: string; dataDir: string }>("hello", {})
      .then(resolve)
      .catch(reject);
  });
}

export function hostCall<T = unknown>(method: string, params: Record<string, unknown>): Promise<T> {
  if (!port) {
    return Promise.reject(new Error("Not connected. Run `doonce setup` and reload the extension."));
  }
  const id = nextId++;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve: resolve as (v: unknown) => void, reject });
    port!.postMessage({ id, method, params });
  });
}

export function driverCall<T = unknown>(driver: string, params: Record<string, unknown>, timeoutMs: number): Promise<T> {
  if (!port) {
    return Promise.reject(new Error("Not connected"));
  }
  const id = `h${nextId++}`;
  return new Promise((resolve, reject) => {
    driverPending.set(id, { resolve: resolve as (v: unknown) => void, reject });
    port!.postMessage({ id, driver, params });
    setTimeout(() => {
      if (driverPending.has(id)) {
        driverPending.delete(id);
        reject(Object.assign(new Error(`Timed out (${driver})`), { transient: true }));
      }
    }, timeoutMs + 5000);
  });
}
