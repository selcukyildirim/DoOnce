// Deterministic page actions used during replay. Each action re-locates its
// target, so the result never depends on stale element references.

import { isTextField, isToggle, normalize } from "../shared/dom";
import type { Condition, Target } from "../shared/types";
import { locate } from "./locate";

export interface PerformArgs {
  action: "click" | "fill" | "select" | "check" | "press";
  target?: Target;
  value?: string;
  checked?: boolean;
  key?: string;
}

/**
 * `dispatched` tells the engine whether the page may already have reacted.
 * Undispatched failures are safe to retry; dispatched ones are not.
 */
export type PerformResult =
  | { ok: true; openUrl?: string }
  | { ok: false; code: string; message: string; dispatched: boolean };

function fail(code: string, message: string, dispatched = false): PerformResult {
  return { ok: false, code, message, dispatched };
}

function resolve(target: Target | undefined): Element | PerformResult {
  if (!target) return fail("noTarget", "step has no target");
  const found = locate(target);
  if (found.status !== "found" || !found.element) {
    return fail(found.status, `element ${found.status === "notFound" ? "not found" : "is " + found.status}`);
  }
  return found.element;
}

function centerIsReachable(el: Element): boolean {
  const rect = el.getBoundingClientRect();
  if (!rect.width && !rect.height) return true;
  const doc = el.ownerDocument;
  if (typeof doc.elementFromPoint !== "function") return true;
  const x = rect.left + rect.width / 2;
  const y = rect.top + rect.height / 2;
  const view = doc.defaultView;
  if (view && (x < 0 || y < 0 || x > view.innerWidth || y > view.innerHeight)) return true;
  const hit = doc.elementFromPoint(x, y);
  return !hit || el === hit || el.contains(hit) || hit.contains(el) || (el instanceof HTMLInputElement && !!el.labels &&
    Array.from(el.labels).some((l) => l.contains(hit)));
}

function prepare(el: Element): PerformResult | null {
  (el as HTMLElement).scrollIntoView?.({ block: "center", inline: "center" });
  if (!centerIsReachable(el)) return fail("obscured", "element is covered by another element");
  return null;
}

function mouse(el: Element, type: string) {
  const view = el.ownerDocument.defaultView;
  const init = { bubbles: true, cancelable: true, composed: true, view, button: 0 };
  const Ctor = type.startsWith("pointer") && typeof PointerEvent !== "undefined" ? PointerEvent : MouseEvent;
  el.dispatchEvent(new Ctor(type, init));
}

function click(el: Element) {
  mouse(el, "pointerdown");
  mouse(el, "mousedown");
  (el as HTMLElement).focus?.({ preventScroll: true });
  mouse(el, "pointerup");
  mouse(el, "mouseup");
  // click() runs the element's activation behavior (links, submit buttons, toggles).
  (el as HTMLElement).click();
}

function setNativeValue(el: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement, value: string) {
  // Bypass framework-installed value setters (React tracks values this way).
  const proto = Object.getPrototypeOf(el);
  const setter = Object.getOwnPropertyDescriptor(proto, "value")?.set;
  if (setter) setter.call(el, value);
  else el.value = value;
}

function fireInput(el: Element) {
  el.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
  el.dispatchEvent(new Event("change", { bubbles: true }));
}

function fill(el: Element, value: string): PerformResult {
  if ((el as HTMLElement).isContentEditable) {
    (el as HTMLElement).focus();
    el.textContent = value;
    el.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
    return { ok: true };
  }
  if (!isTextField(el)) return fail("notEditable", "element is not a text field");
  if (el.readOnly) return fail("readOnly", "field is read-only");
  el.focus({ preventScroll: true });
  setNativeValue(el, value);
  fireInput(el);
  if (el.value !== value && !(el instanceof HTMLInputElement && el.type === "number")) {
    return fail("rejected", "the page did not accept the value", true);
  }
  return { ok: true };
}

function select(el: Element, value: string): PerformResult {
  if (!(el instanceof HTMLSelectElement)) return fail("notSelect", "element is not a dropdown");
  const options = Array.from(el.options);
  const wanted = normalize(value).toLowerCase();
  const option = options.find((o) => o.value === value) ??
    options.find((o) => normalize(o.label || o.text).toLowerCase() === wanted);
  if (!option) return fail("noOption", "the dropdown has no matching option");
  el.focus({ preventScroll: true });
  setNativeValue(el, option.value);
  fireInput(el);
  return { ok: true };
}

function check(el: Element, checked: boolean): PerformResult {
  const input = el;
  if (!isToggle(input)) {
    const role = el.getAttribute("role");
    if (role === "checkbox" || role === "radio" || role === "switch") {
      const current = el.getAttribute("aria-checked") === "true";
      if (current !== checked) click(el);
      return { ok: true };
    }
    return fail("notToggle", "element is not a checkbox or radio button");
  }
  if (input.checked === checked) return { ok: true };
  if (input.type === "radio" && !checked) return fail("radio", "a radio button cannot be unchecked directly");
  click(input);
  return input.checked === checked ? { ok: true } : fail("rejected", "the page did not change the checkbox", true);
}

export function parseKey(spec: string) {
  const parts = spec.split("+").map((p) => p.trim()).filter(Boolean);
  const key = parts.pop() ?? "";
  const mods = new Set(parts.map((p) => p.toLowerCase()));
  const code = key.length === 1
    ? (/[a-z]/i.test(key) ? `Key${key.toUpperCase()}` : /\d/.test(key) ? `Digit${key}` : key)
    : key;
  return {
    key,
    code,
    ctrlKey: mods.has("control") || mods.has("ctrl"),
    shiftKey: mods.has("shift"),
    altKey: mods.has("alt") || mods.has("option"),
    metaKey: mods.has("meta") || mods.has("cmd") || mods.has("command"),
  };
}

function press(el: Element, spec: string): PerformResult {
  const init = { ...parseKey(spec), bubbles: true, cancelable: true, composed: true };
  (el as HTMLElement).focus?.({ preventScroll: true });
  const proceed = el.dispatchEvent(new KeyboardEvent("keydown", init));
  if (init.key.length === 1) el.dispatchEvent(new KeyboardEvent("keypress", init));
  el.dispatchEvent(new KeyboardEvent("keyup", init));
  if (!proceed || init.key !== "Enter" || init.ctrlKey || init.metaKey || init.altKey) return { ok: true };
  // Synthetic key events have no default action; emulate Enter.
  if (el instanceof HTMLInputElement && el.form && isTextField(el)) {
    const form = el.form;
    const submitter = form.querySelector<HTMLElement>("button:not([type]), button[type=submit], input[type=submit]");
    if (submitter) submitter.click();
    else form.requestSubmit();
  } else if (el instanceof HTMLButtonElement || el instanceof HTMLAnchorElement || el.getAttribute("role") === "button") {
    (el as HTMLElement).click();
  }
  return { ok: true };
}

export function perform(args: PerformArgs): PerformResult {
  let el: Element;
  if (args.action === "press" && !args.target) {
    el = document.activeElement ?? document.body;
  } else {
    const resolved = resolve(args.target);
    if (!(resolved instanceof Element)) return resolved;
    el = resolved;
  }
  const blocked = args.action === "press" ? null : prepare(el);
  if (blocked) return blocked;
  try {
    switch (args.action) {
      case "click": {
        // Scripted clicks on target=_blank links are stopped by the popup
        // blocker, so the driver opens the tab instead.
        const link = el.closest("a[href]") as HTMLAnchorElement | null;
        if (link && link.target && !["_self", "_parent", "_top"].includes(link.target) &&
          /^https?:/.test(link.href)) {
          return { ok: true, openUrl: link.href };
        }
        click(el);
        return { ok: true };
      }
      case "fill":
        return fill(el, args.value ?? "");
      case "select":
        return select(el, args.value ?? "");
      case "check":
        return check(el, args.checked ?? true);
      case "press":
        return press(el, args.key ?? "Enter");
    }
  } catch (e) {
    return fail("exception", e instanceof Error ? e.message.slice(0, 200) : "action failed", true);
  }
  return fail("unknownAction", "unknown action");
}

export function checkCondition(condition: Condition): { ok: boolean; failed?: string } {
  if (condition.urlContains && !location.href.includes(condition.urlContains)) return { ok: false, failed: "urlContains" };
  if (condition.textContains) {
    const text = normalize((document.body as HTMLElement | null)?.innerText ?? document.body?.textContent);
    if (!text.toLowerCase().includes(normalize(condition.textContains).toLowerCase())) {
      return { ok: false, failed: "textContains" };
    }
  }
  if (condition.visible) {
    const r = locate(condition.visible);
    if (r.status !== "found" && r.status !== "disabled") return { ok: false, failed: "visible" };
  }
  if (condition.hidden) {
    const r = locate(condition.hidden);
    if (r.status === "found" || r.status === "disabled") return { ok: false, failed: "hidden" };
  }
  return { ok: true };
}
