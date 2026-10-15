import type { BgOk, BgRequest, BgResponse } from "../shared/messages";

export async function bg(req: BgRequest): Promise<BgOk> {
  const res = (await chrome.runtime.sendMessage(req)) as BgResponse;
  if (!res?.ok) throw new Error(res.error || "Request failed");
  return res;
}

export function onBackground(
  fn: (msg: { type: string; execution?: unknown; event?: unknown }) => void,
): () => void {
  const listener = (msg: { type?: string }) => {
    if (msg?.type) fn(msg as never);
  };
  chrome.runtime.onMessage.addListener(listener);
  return () => chrome.runtime.onMessage.removeListener(listener);
}
