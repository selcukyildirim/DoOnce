// Entry point of recorder.js. Injected into the recorded tab only while a
// recording is active. Turns trusted user interactions into compact events:
// typing becomes a single "fill", sensitive values never leave the page.

import { describeElement, isSensitiveField, isTextField, isToggle, tagOf } from "../shared/dom";
import type { RecordedKind } from "../shared/types";

export type Sink = (event: RecordedKind & { ts: number; url: string }) => void;

const ACTIONABLE = [
  "button", "a[href]", "summary", "input[type=submit]", "input[type=button]", "input[type=reset]",
  "input[type=image]", "[role=button]", "[role=link]", "[role=menuitem]", "[role=tab]", "[role=option]",
  "[role=switch]", "[onclick]",
].join(", ");

/** Editing shortcuts that are not meaningful workflow steps. */
const IGNORED_SHORTCUTS = new Set(["a", "c", "v", "x", "z", "y"]);

interface Recorder {
  start(sink: Sink): void;
  stop(): void;
  active(): boolean;
}

function createRecorder(): Recorder {
  let sink: Sink | null = null;
  let indicator: HTMLElement | null = null;
  // Last value emitted per field, so change events after Enter are not duplicated.
  const emitted = new WeakMap<Element, string>();
  const dirty = new Set<HTMLInputElement | HTMLTextAreaElement>();

  const emit = (event: RecordedKind) => sink?.({ ...event, ts: Date.now(), url: location.href });

  const isOurs = (el: EventTarget | null) => el instanceof Node && !!indicator && indicator.contains(el);

  function flushFill(el: HTMLInputElement | HTMLTextAreaElement) {
    dirty.delete(el);
    const sensitive = isSensitiveField(el);
    // For sensitive fields only a marker is stored, never the value.
    const marker = sensitive ? `•${el.value.length}` : el.value;
    if (emitted.get(el) === marker) return;
    emitted.set(el, marker);
    emit({ kind: "fill", target: describeElement(el), value: sensitive ? null : el.value, sensitive });
  }

  function onClick(e: MouseEvent) {
    if (!e.isTrusted || isOurs(e.target) || !(e.target instanceof Element)) return;
    const raw = e.target;
    // Form controls are recorded through their change events instead.
    const control = raw.closest("input, select, textarea, label, option");
    if (control) {
      const input = control instanceof HTMLLabelElement ? control.control : control;
      if (input && (isTextField(input) || isToggle(input) || input instanceof HTMLSelectElement ||
        tagOf(input) === "option")) return;
    }
    const el = raw.closest(ACTIONABLE) ?? raw;
    if (el === document.body || el === document.documentElement) return;
    emit({ kind: "click", target: describeElement(el) });
  }

  function onInput(e: Event) {
    if (!e.isTrusted) return;
    const el = e.target;
    if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) {
      if (isTextField(el)) dirty.add(el);
    }
  }

  function onChange(e: Event) {
    if (!e.isTrusted) return;
    const el = e.target;
    if (!(el instanceof Element)) return;
    if (isToggle(el)) {
      emit({ kind: "check", target: describeElement(el), checked: el.checked });
    } else if (el instanceof HTMLSelectElement) {
      emit({ kind: "select", target: describeElement(el), value: el.value });
    } else if (isTextField(el)) {
      flushFill(el);
    }
  }

  function onKeyDown(e: KeyboardEvent) {
    if (!e.isTrusted || e.repeat || isOurs(e.target)) return;
    const el = e.target instanceof Element ? e.target : null;
    const editable = !!el && (isTextField(el) || (el as HTMLElement).isContentEditable);
    const hasModifier = e.ctrlKey || e.metaKey || e.altKey;
    if (e.key === "Enter" && el && !hasModifier) {
      if (el instanceof HTMLTextAreaElement) return;
      if (isTextField(el)) {
        flushFill(el);
        emit({ kind: "press", target: describeElement(el), key: "Enter" });
      }
      return;
    }
    if (e.key === "Escape") {
      emit({ kind: "press", key: "Escape" });
      return;
    }
    if (hasModifier && !["Control", "Meta", "Alt", "Shift"].includes(e.key)) {
      if (editable && IGNORED_SHORTCUTS.has(e.key.toLowerCase())) return;
      const mods = [e.ctrlKey && "Control", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Meta"]
        .filter(Boolean).join("+");
      const key = e.key.length === 1 ? e.key.toUpperCase() : e.key;
      emit({ kind: "press", key: `${mods}+${key}` });
    }
  }

  function onPageHide() {
    dirty.forEach(flushFill);
  }

  function showIndicator() {
    const host = document.createElement("doonce-indicator");
    const shadow = host.attachShadow({ mode: "closed" });
    shadow.innerHTML = `<style>
      :host { all: initial; position: fixed; z-index: 2147483647; right: 16px; bottom: 16px; pointer-events: none; }
      div { font: 600 12px -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; color: #fff;
        background: rgba(28,28,30,.88); border-radius: 999px; padding: 7px 12px 7px 10px; display: flex;
        align-items: center; gap: 7px; box-shadow: 0 4px 16px rgba(0,0,0,.2); backdrop-filter: blur(8px); }
      i { width: 8px; height: 8px; border-radius: 50%; background: #ff453a; animation: p 1.4s infinite; }
      @keyframes p { 50% { opacity: .35; } }
    </style><div><i></i>DoOnce is recording</div>`;
    document.documentElement.appendChild(host);
    indicator = host;
  }

  const listeners: [string, EventListener][] = [
    ["click", onClick as EventListener],
    ["input", onInput],
    ["change", onChange],
    ["keydown", onKeyDown as EventListener],
  ];

  return {
    start(next) {
      if (sink) return;
      sink = next;
      listeners.forEach(([type, fn]) => window.addEventListener(type, fn, true));
      window.addEventListener("pagehide", onPageHide, true);
      showIndicator();
    },
    stop() {
      if (!sink) return;
      dirty.forEach(flushFill);
      listeners.forEach(([type, fn]) => window.removeEventListener(type, fn, true));
      window.removeEventListener("pagehide", onPageHide, true);
      indicator?.remove();
      indicator = null;
      sink = null;
    },
    active: () => sink !== null,
  };
}

const g = globalThis as { __doonceRecorder?: Recorder };
g.__doonceRecorder ??= createRecorder();
