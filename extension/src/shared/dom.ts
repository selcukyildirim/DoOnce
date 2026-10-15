// DOM inspection helpers shared by the recorder and the replay page agent.
// They run inside arbitrary web pages, so they never trust page content and
// never read field values except where explicitly noted.

import type { Target } from "./types";

const MAX_TEXT = 120;

export function normalize(text: string | null | undefined): string {
  return (text ?? "").replace(/\s+/g, " ").trim();
}

function clip(text: string, max = MAX_TEXT): string {
  return text.length > max ? text.slice(0, max - 1) + "…" : text;
}

export function tagOf(el: Element): string {
  return el.tagName.toLowerCase();
}

const TEXT_INPUT_TYPES = new Set(["", "text", "email", "password", "search", "tel", "url", "number", "date",
  "datetime-local", "month", "time", "week", "color"]);

export function isTextField(el: Element): el is HTMLInputElement | HTMLTextAreaElement {
  if (el instanceof HTMLTextAreaElement) return true;
  return el instanceof HTMLInputElement && TEXT_INPUT_TYPES.has(el.type.toLowerCase());
}

export function isToggle(el: Element): el is HTMLInputElement {
  return el instanceof HTMLInputElement && (el.type === "checkbox" || el.type === "radio");
}

export function implicitRole(el: Element): string | undefined {
  const explicit = el.getAttribute("role");
  if (explicit) return explicit.split(/\s+/)[0];
  const tag = tagOf(el);
  switch (tag) {
    case "a":
      return el.hasAttribute("href") ? "link" : undefined;
    case "button":
      return "button";
    case "select":
      return (el as HTMLSelectElement).multiple || (el as HTMLSelectElement).size > 1 ? "listbox" : "combobox";
    case "textarea":
      return "textbox";
    case "h1": case "h2": case "h3": case "h4": case "h5": case "h6":
      return "heading";
    case "img":
      return "img";
    case "option":
      return "option";
    case "summary":
      return "button";
    case "input": {
      const type = (el as HTMLInputElement).type.toLowerCase();
      if (["button", "submit", "reset", "image"].includes(type)) return "button";
      if (type === "checkbox") return "checkbox";
      if (type === "radio") return "radio";
      if (type === "range") return "slider";
      if (type === "number") return "spinbutton";
      if (type === "search") return "searchbox";
      return "textbox";
    }
  }
  return undefined;
}

/** CSS selector for elements that may have the given role. */
export function roleQuery(role: string): string {
  const implicit: Record<string, string> = {
    button: "button, summary, input[type=button], input[type=submit], input[type=reset], input[type=image]",
    link: "a[href]",
    textbox: "textarea, input:not([type]), input[type=text], input[type=email], input[type=password], input[type=tel], input[type=url], input[type=date]",
    searchbox: "input[type=search]",
    combobox: "select",
    listbox: "select",
    checkbox: "input[type=checkbox]",
    radio: "input[type=radio]",
    spinbutton: "input[type=number]",
    heading: "h1, h2, h3, h4, h5, h6",
  };
  const parts = [`[role="${role.replace(/"/g, "")}"]`];
  if (implicit[role]) parts.push(implicit[role]);
  return parts.join(", ");
}

function textOfIds(ids: string, doc: Document): string {
  return normalize(ids.split(/\s+/).map((id) => doc.getElementById(id)?.textContent ?? "").join(" "));
}

export function labelText(el: Element): string {
  const labels = (el as HTMLInputElement).labels;
  if (labels && labels.length) {
    return normalize(Array.from(labels).map((l) => visibleText(l, el)).join(" "));
  }
  return "";
}

/** Text of `el` excluding the text of `exclude` (e.g. a field inside its label). */
function visibleText(el: Element, exclude?: Element): string {
  if (!exclude || !el.contains(exclude)) return normalize(el.textContent);
  const clone = el.cloneNode(true) as Element;
  clone.querySelectorAll("input, select, textarea").forEach((n) => n.remove());
  return normalize(clone.textContent);
}

/** A simplified accessible-name computation (never uses field values). */
export function accessibleName(el: Element): string {
  const doc = el.ownerDocument;
  const labelledBy = el.getAttribute("aria-labelledby");
  if (labelledBy) {
    const t = textOfIds(labelledBy, doc);
    if (t) return t;
  }
  const aria = normalize(el.getAttribute("aria-label"));
  if (aria) return aria;
  if (el instanceof HTMLInputElement && ["button", "submit", "reset"].includes(el.type)) {
    return normalize(el.value) || (el.type === "submit" ? "Submit" : "");
  }
  if (el instanceof HTMLInputElement && el.type === "image") return normalize(el.alt);
  if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement) {
    return labelText(el) || normalize(el.getAttribute("placeholder")) || normalize(el.getAttribute("title"));
  }
  if (el instanceof HTMLImageElement) return normalize(el.alt);
  const role = implicitRole(el);
  if (role && ["button", "link", "heading", "tab", "menuitem", "option", "checkbox", "radio", "switch"].includes(role)) {
    const text = normalize(el.textContent);
    if (text) return text;
    const img = el.querySelector("img[alt]");
    if (img) return normalize(img.getAttribute("alt"));
  }
  return normalize(el.getAttribute("title"));
}

const HEADING_SELECTOR = "h1, h2, h3, h4, h5, h6, [role=heading], legend";

/** Nearby heading or landmark label used to tell similar elements apart. */
export function contextText(el: Element): string | undefined {
  let node = el.parentElement;
  for (let depth = 0; node && depth < 8; depth++, node = node.parentElement) {
    const tag = tagOf(node);
    if (tag === "body" || tag === "html") break;
    const label = node.getAttribute("aria-label");
    if (label && ["section", "form", "fieldset", "nav", "aside", "dialog", "article"].includes(tag)) {
      return clip(normalize(label));
    }
    const labelledBy = node.getAttribute("aria-labelledby");
    if (labelledBy) {
      const t = textOfIds(labelledBy, node.ownerDocument);
      if (t) return clip(t);
    }
    for (const child of Array.from(node.children)) {
      if (child.contains(el)) break;
      const heading = child.matches(HEADING_SELECTOR) ? child : child.querySelector(HEADING_SELECTOR);
      if (heading) {
        const t = normalize(heading.textContent);
        if (t) return clip(t);
      }
    }
  }
  return undefined;
}

export function isVisible(el: Element): boolean {
  const anyEl = el as Element & { checkVisibility?: (o?: object) => boolean };
  if (typeof anyEl.checkVisibility === "function") {
    if (!anyEl.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true })) return false;
  } else {
    const style = el.ownerDocument.defaultView?.getComputedStyle(el);
    if (style && (style.display === "none" || style.visibility === "hidden" || style.opacity === "0")) return false;
  }
  return el.getClientRects().length > 0;
}

export function isDisabled(el: Element): boolean {
  return (el as HTMLButtonElement).disabled === true || el.getAttribute("aria-disabled") === "true";
}

const SENSITIVE_AUTOCOMPLETE = /^(current-password|new-password|one-time-code|cc-|.*\bcc-)/;
const SENSITIVE_NAME = /pass(word|wd|code)?|pwd|secret|token|otp|2fa|mfa|cvv|cvc|csc|card.?(num|no)|cc.?num|iban|ssn|social.?security|\bpin\b|security.?code|api.?key/i;

/** Fields whose value must never leave the page. */
export function isSensitiveField(el: Element): boolean {
  if (el instanceof HTMLInputElement && el.type === "password") return true;
  const autocomplete = (el.getAttribute("autocomplete") ?? "").toLowerCase();
  if (SENSITIVE_AUTOCOMPLETE.test(autocomplete)) return true;
  if (el.hasAttribute("data-doonce-sensitive")) return true;
  const hints = [el.getAttribute("name"), el.id, el.getAttribute("aria-label"), el.getAttribute("placeholder"),
    labelText(el)].filter(Boolean).join(" ");
  return SENSITIVE_NAME.test(hints);
}

export function cssEscape(value: string): string {
  const css = (globalThis as { CSS?: { escape?: (s: string) => string } }).CSS;
  if (css?.escape) return css.escape(value);
  return value.replace(/([^a-zA-Z0-9_-])/g, "\\$1");
}

function attrSelector(tag: string, name: string, value: string): string {
  return `${tag}[${name}="${value.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"]`;
}

/** Ids and classes that look generated (React ids, CSS-in-JS hashes) change between builds. */
export function looksGenerated(token: string): boolean {
  return /^(:r|css-|sc-|jsx-|emotion-|ember\d|ng-|mui-|_)/.test(token) || /\d{3,}/.test(token) ||
    /^[a-z0-9]{6,}$/i.test(token) && /\d/.test(token) && /[a-z]/i.test(token);
}

const TEST_ATTRIBUTES = ["data-testid", "data-test", "data-qa", "data-cy", "data-test-id"];

function unique(doc: Document, selector: string, el: Element): boolean {
  try {
    const all = doc.querySelectorAll(selector);
    return all.length === 1 && all[0] === el;
  } catch {
    return false;
  }
}

function structuralPath(el: Element): string {
  const parts: string[] = [];
  let node: Element | null = el;
  while (node && node.tagName && tagOf(node) !== "html" && parts.length < 6) {
    const tag = tagOf(node);
    if (node.id && !looksGenerated(node.id) && node !== el) {
      parts.unshift(`#${cssEscape(node.id)}`);
      break;
    }
    const parent: Element | null = node.parentElement;
    if (!parent) {
      parts.unshift(tag);
      break;
    }
    const siblings = Array.from(parent.children).filter((c) => c.tagName === node!.tagName);
    parts.unshift(siblings.length > 1 ? `${tag}:nth-of-type(${siblings.indexOf(node) + 1})` : tag);
    node = parent;
  }
  return parts.join(" > ");
}

export function candidateSelectors(el: Element): string[] {
  const doc = el.ownerDocument;
  const tag = tagOf(el);
  const out: string[] = [];
  const add = (s: string) => {
    if (out.length < 5 && !out.includes(s) && unique(doc, s, el)) out.push(s);
  };
  for (const attr of TEST_ATTRIBUTES) {
    const v = el.getAttribute(attr);
    if (v) add(attrSelector("", attr, v));
  }
  if (el.id && !looksGenerated(el.id)) add(`#${cssEscape(el.id)}`);
  const name = el.getAttribute("name");
  if (name) add(attrSelector(tag, "name", name));
  const aria = el.getAttribute("aria-label");
  if (aria) add(attrSelector(tag, "aria-label", aria));
  const classes = Array.from(el.classList).filter((c) => !looksGenerated(c)).slice(0, 3);
  if (classes.length) add(`${tag}.${classes.map(cssEscape).join(".")}`);
  if (el instanceof HTMLAnchorElement) {
    const href = el.getAttribute("href");
    if (href && !href.startsWith("javascript:") && href.length < 200) add(attrSelector("a", "href", href));
  }
  const path = structuralPath(el);
  if (path) add(path);
  return out;
}

const KEPT_ATTRIBUTES = ["id", "name", "type", "placeholder", "aria-label", "title", "alt", "for", "role",
  "autocomplete", "href", ...TEST_ATTRIBUTES, "data-action"];

/** Everything the replay engine needs to find `el` again. Never includes field values. */
export function describeElement(el: Element): Target {
  const attributes: Record<string, string> = {};
  for (const name of KEPT_ATTRIBUTES) {
    const value = el.getAttribute(name);
    if (value && value.length <= 300 && !(name === "href" && value.startsWith("javascript:"))) {
      attributes[name] = value;
    }
  }
  const stableClasses = Array.from(el.classList).filter((c) => !looksGenerated(c));
  if (stableClasses.length) attributes["class"] = stableClasses.slice(0, 4).join(" ");

  const target: Target = { tag: tagOf(el), attributes, selectors: candidateSelectors(el) };
  const role = implicitRole(el);
  if (role) target.role = role;
  const name = accessibleName(el);
  if (name) target.name = clip(name);
  const label = labelText(el);
  if (label) target.label = clip(label);
  if (!isTextField(el) && !(el instanceof HTMLSelectElement)) {
    const text = normalize(el.textContent);
    if (text && text !== target.name) target.text = clip(text);
  }
  const context = contextText(el);
  if (context) target.context = context;
  return target;
}
