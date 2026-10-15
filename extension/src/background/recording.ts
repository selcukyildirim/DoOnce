import type { RecordedEvent, RecordedKind } from "../shared/types";

let recording = false;
let tabId: number | null = null;
const events: RecordedEvent[] = [];

export function isRecording(): boolean {
  return recording;
}

export function getEvents(): RecordedEvent[] {
  return [...events];
}

export function clearEvents() {
  events.length = 0;
}

export function pushRecorded(payload: RecordedKind & { ts?: number; url?: string }) {
  if (!recording || tabId == null) return;
  events.push({
    ...payload,
    ts: payload.ts ?? Date.now(),
    url: payload.url,
    tabId,
  });
}

export function pushNavigate(url: string, transition?: string) {
  if (!recording) return;
  pushRecorded({ kind: "navigate", transition, url, ts: Date.now() });
}

export async function startRecording(): Promise<number> {
  if (recording) throw new Error("Already recording");
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (!tab?.id || !tab.url?.startsWith("http")) {
    throw new Error("Open a normal web page (http/https) before recording.");
  }
  tabId = tab.id;
  recording = true;
  events.length = 0;
  const title = tab.title;
  const url = tab.url;
  pushRecorded({ kind: "start", title, url, ts: Date.now() });

  await chrome.scripting.executeScript({ target: { tabId }, files: ["recorder.js"], world: "ISOLATED" });
  await chrome.scripting.executeScript({
    target: { tabId },
    world: "ISOLATED",
    func: () => {
      const rec = (globalThis as { __doonceRecorder?: { start: (sink: (e: unknown) => void) => void } })
        .__doonceRecorder;
      if (!rec) throw new Error("recorder not loaded");
      rec.start((event) => {
        chrome.runtime.sendMessage({ channel: "recordEvent", payload: event });
      });
    },
  });
  return tabId;
}

export function stopRecording(): RecordedEvent[] {
  if (!recording || tabId == null) return getEvents();
  recording = false;
  chrome.scripting
    .executeScript({
      target: { tabId },
      world: "ISOLATED",
      func: () => (globalThis as { __doonceRecorder?: { stop: () => void } }).__doonceRecorder?.stop(),
    })
    .catch(() => {});
  tabId = null;
  return getEvents();
}
