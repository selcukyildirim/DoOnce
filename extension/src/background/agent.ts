/** Injects and calls the bundled page agent in a tab's isolated world. */

const agentReady = new Set<number>();

async function ensureAgent(tabId: number): Promise<void> {
  if (agentReady.has(tabId)) return;
  await chrome.scripting.executeScript({
    target: { tabId },
    files: ["page-agent.js"],
    world: "ISOLATED",
  });
  agentReady.add(tabId);
}

export async function agentCall(tabId: number, method: string, args: Record<string, unknown>): Promise<unknown> {
  await ensureAgent(tabId);
  const [result] = await chrome.scripting.executeScript({
    target: { tabId },
    world: "ISOLATED",
    func: (m: string, a: Record<string, unknown>) => {
      const api = (globalThis as { __doonce?: { call: (method: string, args: Record<string, unknown>) => unknown } })
        .__doonce;
      if (!api) throw new Error("page agent not loaded");
      return api.call(m, a);
    },
    args: [method, args] as [string, Record<string, unknown>],
  });
  return result?.result;
}

export function forgetTab(tabId: number) {
  agentReady.delete(tabId);
}
