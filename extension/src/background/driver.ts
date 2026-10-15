import { agentCall } from "./agent";

let expectingDownload = false;
let downloadWaiter: {
  resolve: (info: { filename: string; path?: string; bytes?: number }) => void;
  reject: (e: Error) => void;
} | null = null;

chrome.downloads.onChanged.addListener((delta) => {
  if (!expectingDownload || !downloadWaiter || !delta.state) return;
  if (delta.state.current === "complete") {
    chrome.downloads.search({ id: delta.id }, (items) => {
      const item = items[0];
      if (!item || !downloadWaiter) return;
      const w = downloadWaiter;
      downloadWaiter = null;
      expectingDownload = false;
      w.resolve({ filename: item.filename ?? "download", bytes: item.totalBytes ?? undefined });
    });
  } else if (delta.state.current === "interrupted") {
    downloadWaiter?.reject(new Error("download interrupted"));
    downloadWaiter = null;
    expectingDownload = false;
  }
});

function waitTabComplete(tabId: number, timeoutMs: number): Promise<void> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      chrome.tabs.onUpdated.removeListener(onUpdated);
      reject(Object.assign(new Error("navigation timed out"), { transient: true }));
    }, timeoutMs);
    const onUpdated = (id: number, info: { status?: string }) => {
      if (id === tabId && info.status === "complete") {
        clearTimeout(timer);
        chrome.tabs.onUpdated.removeListener(onUpdated);
        resolve();
      }
    };
    chrome.tabs.onUpdated.addListener(onUpdated);
    chrome.tabs.get(tabId).then((tab) => {
      if (tab.status === "complete") {
        clearTimeout(timer);
        chrome.tabs.onUpdated.removeListener(onUpdated);
        resolve();
      }
    });
  });
}

export async function handleDriver(
  op: string,
  params: Record<string, unknown>,
): Promise<unknown> {
  switch (op) {
    case "navigate": {
      const url = String(params.url ?? "");
      const timeoutMs = Number(params.timeoutMs ?? 30000);
      let tabId = params.tabId != null ? Number(params.tabId) : undefined;
      if (tabId) {
        await chrome.tabs.update(tabId, { url });
      } else {
        const tab = await chrome.tabs.create({ url, active: true });
        tabId = tab.id!;
      }
      await waitTabComplete(tabId, timeoutMs);
      return { tabId };
    }
    case "call": {
      const tabId = Number(params.tabId);
      const method = String(params.method);
      const args = (params.args as Record<string, unknown>) ?? {};
      return agentCall(tabId, method, args);
    }
    case "perform": {
      const tabId = Number(params.tabId);
      const args = (params.args as Record<string, unknown>) ?? {};
      const timeoutMs = Number(params.timeoutMs ?? 30000);
      const result = (await agentCall(tabId, "perform", args)) as {
        ok?: boolean;
        openUrl?: string;
        message?: string;
        code?: string;
        dispatched?: boolean;
      };
      if (result?.openUrl && /^https?:/.test(result.openUrl)) {
        const tab = await chrome.tabs.create({ url: result.openUrl, active: true });
        return { ...result, tabId: tab.id };
      }
      if (result?.ok && (args.action === "click" || args.action === "press")) {
        try {
          await waitTabComplete(tabId, Math.min(timeoutMs, 15000));
        } catch {
          /* same-document actions are fine */
        }
      }
      return result;
    }
    case "expectDownload": {
      expectingDownload = true;
      return null;
    }
    case "waitDownload": {
      const timeoutMs = Number(params.timeoutMs ?? 60000);
      if (!expectingDownload) throw new Error("download was not expected");
      return new Promise((resolve, reject) => {
        downloadWaiter = { resolve, reject };
        setTimeout(() => {
          if (downloadWaiter) {
            downloadWaiter.reject(new Error("no download started"));
            downloadWaiter = null;
            expectingDownload = false;
          }
        }, timeoutMs);
      });
    }
    case "switchTab": {
      const urlContains = String(params.urlContains ?? "");
      const timeoutMs = Number(params.timeoutMs ?? 10000);
      const fromTabId = params.fromTabId != null ? Number(params.fromTabId) : undefined;
      const deadline = Date.now() + timeoutMs;
      while (Date.now() < deadline) {
        const tabs = await chrome.tabs.query({});
        const match = tabs.filter(
          (t) => t.url?.includes(urlContains) && t.id != null && (fromTabId == null || t.id !== fromTabId),
        );
        const pick = match[0] ?? tabs.find((t) => t.url?.includes(urlContains));
        if (pick?.id != null) {
          await chrome.tabs.update(pick.id, { active: true });
          return { tabId: pick.id };
        }
        await new Promise((r) => setTimeout(r, 100));
      }
      throw new Error(`no open tab matches "${urlContains}"`);
    }
    default:
      throw new Error(`unknown driver op: ${op}`);
  }
}
