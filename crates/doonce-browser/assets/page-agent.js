// DoOnce page agent.
//
// This file is injected into an isolated world by the Rust CDP driver. It must
// remain dependency-free and must never expose page-provided values in locate
// diagnostics. It is intentionally conservative: deterministic scoring over
// multiple independent signals, no XPath, no generated CSS hashes.
(() => {
  const VERSION = 2;
  const MAX_TEXT = 120;
  const ACCEPT_SCORE = 45;
  const MARGIN = 15;
  const MAX_CANDIDATES = 3000;

  function normalize(text) {
    return String(text == null ? "" : text).replace(/\s+/g, " ").trim();
  }

  function clip(text, max = MAX_TEXT) {
    const value = normalize(text);
    return value.length > max ? value.slice(0, max - 1) + "…" : value;
  }

  function tagOf(el) {
    return el && el.tagName ? el.tagName.toLowerCase() : "";
  }

  const SENSITIVE_AUTOCOMPLETE = /^(current-password|new-password|one-time-code|cc-|.*\bcc-)/;
  const SENSITIVE_NAME = /pass(word|wd|code)?|pwd|secret|token|otp|2fa|mfa|cvv|cvc|csc|card.?(num|no)|cc.?num|iban|ssn|social.?security|\bpin\b|security.?code|api.?key/i;

  function isTextField(el) {
    if (!el || !(el instanceof Element)) return false;
    if (el instanceof HTMLTextAreaElement) return true;
    if (!(el instanceof HTMLInputElement)) return false;
    return new Set(["", "text", "email", "password", "search", "tel", "url", "number", "date", "datetime-local", "month", "time", "week", "color"]).has((el.type || "").toLowerCase());
  }

  function isToggle(el) {
    return el instanceof HTMLInputElement && (el.type === "checkbox" || el.type === "radio");
  }

  function implicitRole(el) {
    const explicit = el.getAttribute && el.getAttribute("role");
    if (explicit) return explicit.split(/\s+/)[0];
    const tag = tagOf(el);
    switch (tag) {
      case "a": return el.hasAttribute("href") ? "link" : undefined;
      case "button": return "button";
      case "select": return el.multiple || el.size > 1 ? "listbox" : "combobox";
      case "textarea": return "textbox";
      case "h1": case "h2": case "h3": case "h4": case "h5": case "h6": return "heading";
      case "img": return "img";
      case "option": return "option";
      case "summary": return "button";
      case "input": {
        const type = (el.type || "").toLowerCase();
        if (["button", "submit", "reset", "image"].includes(type)) return "button";
        if (type === "checkbox") return "checkbox";
        if (type === "radio") return "radio";
        if (type === "range") return "slider";
        if (type === "number") return "spinbutton";
        if (type === "search") return "searchbox";
        return "textbox";
      }
      default: return undefined;
    }
  }

  function roleQuery(role) {
    const implicit = {
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
    const parts = [`[role="${String(role).replace(/"/g, "")}"]`];
    if (implicit[role]) parts.push(implicit[role]);
    return parts.join(", ");
  }

  function textOfIds(ids, doc) {
    return normalize(String(ids).split(/\s+/).map((id) => {
      const el = doc.getElementById(id);
      return el ? el.textContent : "";
    }).join(" "));
  }

  function visibleText(el, exclude) {
    if (!exclude || !el.contains(exclude)) return normalize(el.textContent);
    const clone = el.cloneNode(true);
    clone.querySelectorAll("input, select, textarea").forEach((node) => node.remove());
    return normalize(clone.textContent);
  }

  function labelText(el) {
    const labels = el.labels;
    if (labels && labels.length) return normalize(Array.from(labels).map((label) => visibleText(label, el)).join(" "));
    return "";
  }

  function accessibleName(el) {
    const doc = el.ownerDocument;
    const labelledBy = el.getAttribute("aria-labelledby");
    if (labelledBy) {
      const text = textOfIds(labelledBy, doc);
      if (text) return text;
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

  function contextText(el) {
    let node = el.parentElement;
    for (let depth = 0; node && depth < 8; depth++, node = node.parentElement) {
      const tag = tagOf(node);
      if (tag === "body" || tag === "html") break;
      const label = node.getAttribute("aria-label");
      if (label && ["section", "form", "fieldset", "nav", "aside", "dialog", "article"].includes(tag)) {
        return clip(label);
      }
      const labelledBy = node.getAttribute("aria-labelledby");
      if (labelledBy) {
        const text = textOfIds(labelledBy, node.ownerDocument);
        if (text) return clip(text);
      }
      for (const child of Array.from(node.children)) {
        if (child.contains(el)) break;
        const heading = child.matches(HEADING_SELECTOR) ? child : child.querySelector(HEADING_SELECTOR);
        if (heading) {
          const text = normalize(heading.textContent);
          if (text) return clip(text);
        }
      }
    }
    return undefined;
  }

  function isVisible(el) {
    if (!el || !(el instanceof Element)) return false;
    if (typeof el.checkVisibility === "function") {
      if (!el.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true })) return false;
    } else {
      const style = el.ownerDocument.defaultView && el.ownerDocument.defaultView.getComputedStyle(el);
      if (style && (style.display === "none" || style.visibility === "hidden" || style.opacity === "0")) return false;
    }
    return el.getClientRects().length > 0;
  }

  function isDisabled(el) {
    return el.disabled === true || el.getAttribute("aria-disabled") === "true";
  }

  function looksGenerated(token) {
    return /^(:r|css-|sc-|jsx-|emotion-|ember\d|ng-|mui-|_)/.test(token) ||
      /\d{3,}/.test(token) ||
      (/^[a-z0-9]{6,}$/i.test(token) && /\d/.test(token) && /[a-z]/i.test(token));
  }

  function cssEscape(value) {
    if (globalThis.CSS && typeof globalThis.CSS.escape === "function") return globalThis.CSS.escape(value);
    return String(value).replace(/([^a-zA-Z0-9_-])/g, "\\$1");
  }

  function attrSelector(tag, name, value) {
    return `${tag}[${name}="${String(value).replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"]`;
  }

  function unique(doc, selector, el) {
    try {
      const all = doc.querySelectorAll(selector);
      return all.length === 1 && all[0] === el;
    } catch (_) {
      return false;
    }
  }

  function structuralPath(el) {
    const parts = [];
    let node = el;
    while (node && node.tagName && tagOf(node) !== "html" && parts.length < 6) {
      const tag = tagOf(node);
      if (node.id && !looksGenerated(node.id) && node !== el) {
        parts.unshift(`#${cssEscape(node.id)}`);
        break;
      }
      const parent = node.parentElement;
      if (!parent) {
        parts.unshift(tag);
        break;
      }
      const siblings = Array.from(parent.children).filter((child) => child.tagName === node.tagName);
      parts.unshift(siblings.length > 1 ? `${tag}:nth-of-type(${siblings.indexOf(node) + 1})` : tag);
      node = parent;
    }
    return parts.join(" > ");
  }

  const TEST_ATTRIBUTES = ["data-testid", "data-test", "data-qa", "data-cy", "data-test-id"];

  function candidateSelectors(el) {
    const doc = el.ownerDocument;
    const tag = tagOf(el);
    const out = [];
    const add = (selector) => {
      if (out.length < 5 && !out.includes(selector) && unique(doc, selector, el)) out.push(selector);
    };
    for (const attr of TEST_ATTRIBUTES) {
      const value = el.getAttribute(attr);
      if (value) add(attrSelector("", attr, value));
    }
    if (el.id && !looksGenerated(el.id)) add(`#${cssEscape(el.id)}`);
    const name = el.getAttribute("name");
    if (name) add(attrSelector(tag, "name", name));
    const aria = el.getAttribute("aria-label");
    if (aria) add(attrSelector(tag, "aria-label", aria));
    const classes = Array.from(el.classList).filter((cls) => !looksGenerated(cls)).slice(0, 3);
    if (classes.length) add(`${tag}.${classes.map(cssEscape).join(".")}`);
    if (el instanceof HTMLAnchorElement) {
      const href = el.getAttribute("href");
      if (href && !href.startsWith("javascript:") && href.length < 200) add(attrSelector("a", "href", href));
    }
    const path = structuralPath(el);
    if (path) add(path);
    return out;
  }

  function query(root, selector) {
    try {
      return Array.from(root.querySelectorAll(selector));
    } catch (_) {
      return [];
    }
  }

  function eq(a, b) {
    return !!a && !!b && normalize(a).toLowerCase() === normalize(b).toLowerCase();
  }

  function tokens(text) {
    return new Set(normalize(text).toLowerCase().split(/[^\p{L}\p{N}]+/u).filter(Boolean));
  }

  function overlap(a, b) {
    const left = tokens(a);
    const right = tokens(b);
    if (!left.size || !right.size) return 0;
    let common = 0;
    left.forEach((token) => { if (right.has(token)) common += 1; });
    return common / Math.max(left.size, right.size);
  }

  function gatherCandidates(doc, target, selectorHits) {
    const out = new Set();
    (target.selectors || []).forEach((selector, index) => {
      const matches = query(doc, selector);
      for (const el of matches) {
        out.add(el);
        const strength = matches.length === 1 ? (index === 0 ? 50 : 40) : 8;
        selectorHits.set(el, Math.max(selectorHits.get(el) || 0, strength));
      }
    });
    const attrs = target.attributes || {};
    if (attrs.id) {
      const byId = doc.getElementById(attrs.id);
      if (byId) out.add(byId);
    }
    if (target.role) query(doc, roleQuery(target.role)).forEach((el) => out.add(el));
    if (target.tag && out.size < MAX_CANDIDATES) query(doc, target.tag).slice(0, MAX_CANDIDATES).forEach((el) => out.add(el));
    if (target.label) query(doc, "input, select, textarea").forEach((el) => out.add(el));
    return out;
  }

  function scoreElement(el, target, selectorScore) {
    let score = selectorScore;
    const reasons = selectorScore ? [`selector+${selectorScore}`] : [];
    const add = (points, reason) => {
      score += points;
      reasons.push(`${reason}${points >= 0 ? "+" : ""}${points}`);
    };

    const role = implicitRole(el);
    if (target.role) {
      if (role === target.role) add(15, "role");
      else add(-30, "role");
    }
    if (target.tag && tagOf(el) === target.tag) add(5, "tag");

    if (target.name) {
      const name = accessibleName(el);
      if (eq(name, target.name)) add(30, "name");
      else if (name && overlap(name, target.name) >= 0.6) add(15, "name~");
      else if (name) add(-10, "name");
    }
    if (target.label) {
      const label = labelText(el);
      if (eq(label, target.label)) {
        add(30, "label");
        if (isTextField(el) || el instanceof HTMLSelectElement || isToggle(el)) add(15, "formField");
      } else if (label && overlap(label, target.label) >= 0.6) {
        add(15, "label~");
      }
    }
    if (target.text && eq(normalize(el.textContent), target.text)) add(10, "text");

    const attrs = target.attributes || {};
    const weights = {
      "data-testid": 30, "data-test": 30, "data-qa": 30, "data-cy": 30,
      id: 25, name: 15, "aria-label": 10, placeholder: 10, href: 10,
      type: 5, "data-action": 10, title: 5,
    };
    for (const [key, weight] of Object.entries(weights)) {
      const wanted = attrs[key];
      if (!wanted) continue;
      const have = el.getAttribute(key);
      if (have === wanted) add(weight, key);
      else if (have && (key === "id" || key.startsWith("data-t") || key === "name")) add(-10, key);
    }
    if (target.context && score >= 20) {
      const context = contextText(el);
      if (eq(context, target.context)) add(15, "context");
      else if (context) add(-15, "context");
    }
    return { element: el, score, reasons };
  }

  function summarize(candidate) {
    return {
      score: candidate.score,
      tag: tagOf(candidate.element),
      role: implicitRole(candidate.element),
      name: clip(accessibleName(candidate.element), 80) || undefined,
      context: contextText(candidate.element),
    };
  }

  function hitTestable(el) {
    const rect = el.getBoundingClientRect();
    if (!rect.width && !rect.height) return false;
    const view = el.ownerDocument.defaultView;
    const x = rect.left + rect.width / 2;
    const y = rect.top + rect.height / 2;
    if (view && (x < 0 || y < 0 || x > view.innerWidth || y > view.innerHeight)) return false;
    const hit = el.ownerDocument.elementFromPoint(x, y);
    return !!hit && (el === hit || el.contains(hit) || hit.contains(el));
  }

  function locate(target, doc = document) {
    if (!target) return { status: "notFound", candidates: [] };
    const selectorHits = new Map();
    const pool = gatherCandidates(doc, target, selectorHits);
    const scored = [];
    pool.forEach((el) => scored.push(scoreElement(el, target, selectorHits.get(el) || 0)));
    scored.sort((a, b) => b.score - a.score);

    const visible = scored.filter((candidate) => isVisible(candidate.element));
    const best = visible[0];
    const top = visible.slice(0, 3).map(summarize);
    if (!best || best.score < ACCEPT_SCORE) {
      const hidden = scored[0];
      if (hidden && hidden.score >= ACCEPT_SCORE && !isVisible(hidden.element)) {
        return { status: "hidden", score: hidden.score, reasons: hidden.reasons, candidates: top };
      }
      return { status: "notFound", score: best ? best.score : undefined, candidates: top };
    }
    const second = visible[1];
    if (second && best.score - second.score < MARGIN) {
      const tied = visible.filter((candidate) => best.score - candidate.score < MARGIN);
      const onTop = tied.filter((candidate) => hitTestable(candidate.element));
      if (onTop.length === 1) {
        const pick = onTop[0];
        if (isDisabled(pick.element)) {
          return { status: "disabled", element: pick.element, score: pick.score, reasons: pick.reasons };
        }
        return { status: "found", element: pick.element, score: pick.score, reasons: pick.reasons.concat("onTop") };
      }
      return { status: "ambiguous", score: best.score, reasons: best.reasons, candidates: top };
    }
    if (isDisabled(best.element)) {
      return { status: "disabled", element: best.element, score: best.score, reasons: best.reasons };
    }
    return { status: "found", element: best.element, score: best.score, reasons: best.reasons };
  }

  function htmlSnippet(el) {
    const html = el.outerHTML || "";
    return html.length > 2000 ? html.slice(0, 2000) + "…" : html;
  }

  function snapshot(target, doc = document) {
    const selectorHits = new Map();
    const pool = gatherCandidates(doc, target, selectorHits);
    const scored = [];
    pool.forEach((el) => scored.push(scoreElement(el, target, selectorHits.get(el) || 0)));
    scored.sort((a, b) => b.score - a.score);
    return {
      url: doc.location.href,
      title: doc.title,
      candidates: scored.slice(0, 8).map((candidate) => ({
        score: candidate.score,
        tag: tagOf(candidate.element),
        role: implicitRole(candidate.element),
        name: clip(accessibleName(candidate.element), 80) || undefined,
        html: htmlSnippet(candidate.element),
      })),
    };
  }

  function fail(code, message, dispatched = false) {
    return { ok: false, code, message, dispatched };
  }

  function resolve(target) {
    if (!target) return fail("noTarget", "step has no target");
    const found = locate(target);
    if (found.status !== "found" || !found.element) {
      return fail(found.status, `element ${found.status === "notFound" ? "not found" : "is " + found.status}`);
    }
    return found.element;
  }

  function centerIsReachable(el) {
    const rect = el.getBoundingClientRect();
    if (!rect.width && !rect.height) return true;
    const doc = el.ownerDocument;
    if (typeof doc.elementFromPoint !== "function") return true;
    const x = rect.left + rect.width / 2;
    const y = rect.top + rect.height / 2;
    const view = doc.defaultView;
    if (view && (x < 0 || y < 0 || x > view.innerWidth || y > view.innerHeight)) return true;
    const hit = doc.elementFromPoint(x, y);
    return !hit || el === hit || el.contains(hit) || hit.contains(el) ||
      (el instanceof HTMLInputElement && el.labels && Array.from(el.labels).some((label) => label.contains(hit)));
  }

  function prepare(el) {
    if (el.scrollIntoView) el.scrollIntoView({ block: "center", inline: "center" });
    if (!centerIsReachable(el)) return fail("obscured", "element is covered by another element");
    return null;
  }

  function mouse(el, type) {
    const view = el.ownerDocument.defaultView;
    const init = { bubbles: true, cancelable: true, composed: true, view, button: 0 };
    const Ctor = type.startsWith("pointer") && typeof PointerEvent !== "undefined" ? PointerEvent : MouseEvent;
    el.dispatchEvent(new Ctor(type, init));
  }

  function enter(el) {
    const doc = el.ownerDocument;
    const view = doc.defaultView;
    const rect = el.getBoundingClientRect();
    const point = { clientX: rect.left + rect.width / 2, clientY: rect.top + rect.height / 2 };
    const send = (type, bubbles) => {
      const init = { ...point, bubbles, cancelable: true, composed: true, view, button: 0 };
      const Ctor = type.startsWith("pointer") && typeof PointerEvent !== "undefined" ? PointerEvent : MouseEvent;
      el.dispatchEvent(new Ctor(type, init));
    };
    send("pointerover", true);
    send("mouseover", true);
    send("pointerenter", false);
    send("mouseenter", false);
    send("pointermove", true);
    send("mousemove", true);
  }

  function click(el) {
    enter(el);
    mouse(el, "pointerdown");
    mouse(el, "mousedown");
    if (el.focus) el.focus({ preventScroll: true });
    mouse(el, "pointerup");
    mouse(el, "mouseup");
    el.click();
  }

  function hover(el) {
    enter(el);
    const rect = el.getBoundingClientRect();
    return { ok: true, rect: { x: rect.left, y: rect.top, width: rect.width, height: rect.height } };
  }

  function setNativeValue(el, value) {
    const proto = Object.getPrototypeOf(el);
    const setter = Object.getOwnPropertyDescriptor(proto, "value");
    if (setter && setter.set) setter.set.call(el, value);
    else el.value = value;
  }

  function fireInput(el) {
    el.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
  }

  function fill(el, value) {
    if (el.isContentEditable) {
      el.focus();
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

  function select(el, value) {
    if (!(el instanceof HTMLSelectElement)) return fail("notSelect", "element is not a dropdown");
    const options = Array.from(el.options);
    const wanted = normalize(value).toLowerCase();
    const option = options.find((item) => item.value === value) ||
      options.find((item) => normalize(item.label || item.text).toLowerCase() === wanted);
    if (!option) return fail("noOption", "the dropdown has no matching option");
    el.focus({ preventScroll: true });
    setNativeValue(el, option.value);
    fireInput(el);
    return { ok: true };
  }

  function check(el, checked) {
    if (!isToggle(el)) {
      const role = el.getAttribute("role");
      if (role === "checkbox" || role === "radio" || role === "switch") {
        const current = el.getAttribute("aria-checked") === "true";
        if (current !== checked) click(el);
        return { ok: true };
      }
      return fail("notToggle", "element is not a checkbox or radio button");
    }
    if (el.checked === checked) return { ok: true };
    if (el.type === "radio" && !checked) return fail("radio", "a radio button cannot be unchecked directly");
    click(el);
    return el.checked === checked ? { ok: true } : fail("rejected", "the page did not change the checkbox", true);
  }

  function parseKey(spec) {
    const parts = String(spec || "").split("+").map((part) => part.trim()).filter(Boolean);
    const key = parts.pop() || "";
    const mods = new Set(parts.map((part) => part.toLowerCase()));
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

  function press(el, spec) {
    const init = { ...parseKey(spec), bubbles: true, cancelable: true, composed: true };
    if (el.focus) el.focus({ preventScroll: true });
    const proceed = el.dispatchEvent(new KeyboardEvent("keydown", init));
    if (init.key.length === 1) el.dispatchEvent(new KeyboardEvent("keypress", init));
    el.dispatchEvent(new KeyboardEvent("keyup", init));
    if (!proceed || init.key !== "Enter" || init.ctrlKey || init.metaKey || init.altKey) return { ok: true };
    if (el instanceof HTMLInputElement && el.form && isTextField(el)) {
      const form = el.form;
      const submitter = form.querySelector("button:not([type]), button[type=submit], input[type=submit]");
      if (submitter) submitter.click();
      else form.requestSubmit();
    } else if (el instanceof HTMLButtonElement || el instanceof HTMLAnchorElement || el.getAttribute("role") === "button") {
      el.click();
    }
    return { ok: true };
  }

  function perform(args) {
    let el;
    if (args.action === "press" && !args.target) {
      el = document.activeElement || document.body;
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
          const link = el.closest("a[href]");
          if (link && link.target && !["_self", "_parent", "_top"].includes(link.target) && /^https?:/.test(link.href)) {
            return { ok: true, openUrl: link.href };
          }
          click(el);
          return { ok: true };
        }
        case "hover": return hover(el);
        case "fill": return fill(el, args.value == null ? "" : String(args.value));
        case "select": return select(el, args.value == null ? "" : String(args.value));
        case "check": return check(el, args.checked !== false);
        case "press": return press(el, args.key || "Enter");
        default: return fail("unknownAction", "unknown action");
      }
    } catch (error) {
      return fail("exception", error instanceof Error ? error.message.slice(0, 200) : "action failed", true);
    }
  }

  function checkCondition(condition) {
    if (!condition) return { ok: true };
    if (condition.urlContains && !location.href.includes(condition.urlContains)) return { ok: false, failed: "urlContains" };
    if (condition.textContains) {
      const text = normalize(document.body ? document.body.innerText : "");
      if (!text.toLowerCase().includes(normalize(condition.textContains).toLowerCase())) {
        return { ok: false, failed: "textContains" };
      }
    }
    if (condition.visible) {
      const result = locate(condition.visible);
      if (result.status !== "found" && result.status !== "disabled") return { ok: false, failed: "visible" };
    }
    if (condition.hidden) {
      const result = locate(condition.hidden);
      if (result.status === "found" || result.status === "disabled") return { ok: false, failed: "hidden" };
    }
    return { ok: true };
  }

  function extract(args) {
    const result = locate(args.target);
    if (result.status !== "found" || !result.element) return { status: result.status };
    const el = result.element;
    const attribute = args.attribute;
    let value = "";
    if (attribute === "value") value = el.value == null ? "" : String(el.value);
    else if (attribute && attribute !== "text") value = el.getAttribute(attribute) || "";
    else value = normalize(el.innerText != null ? el.innerText : el.textContent);
    return { status: "found", value };
  }

  function scroll(args) {
    const direction = String(args.direction || "down");
    const amount = Number(args.amount || 0) || 0;
    const before = window.scrollY;
    const step = amount || Math.round(window.innerHeight * 0.8);
    if (direction === "up") window.scrollBy(0, -step);
    else if (direction === "top") window.scrollTo(0, 0);
    else if (direction === "bottom") window.scrollTo(0, document.body.scrollHeight);
    else window.scrollBy(0, step);
    const max = Math.max(document.body.scrollHeight - window.innerHeight, 0);
    return { y: window.scrollY, before, atBottom: window.scrollY >= max - 2 };
  }

  function zoneOf(el) {
    const zone = el.closest("header, nav, footer, aside, main, [role=main], [role=navigation], [role=banner], [role=contentinfo]");
    if (!zone) return "main";
    const tag = tagOf(zone);
    const role = zone.getAttribute("role");
    if (tag === "header" || role === "banner") return "header";
    if (tag === "nav" || role === "navigation") return "nav";
    if (tag === "footer" || role === "contentinfo") return "footer";
    if (tag === "aside") return "aside";
    if (tag === "main" || role === "main") return "main";
    return "section";
  }

  function observe() {
    const actionable = [
      "a[href]", "button", "input:not([type=hidden])", "select", "textarea", "summary",
      "[role=button]", "[role=link]", "[role=menuitem]", "[role=tab]", "[role=option]",
      "[role=combobox]", "[role=radio]", "[role=switch]", "[role=checkbox]", "[onclick]", "[tabindex]",
    ].join(", ");
    const candidates = document.querySelectorAll(
      "a[href], button, input:not([type=hidden]), select, textarea, summary, [role], [tabindex], [onclick], div, span, li, label, option"
    );
    const main = [];
    const peripheral = [];
    let scanned = 0;
    for (const el of candidates) {
      if (main.length >= 120 && peripheral.length >= 30) break;
      if (scanned++ > 4000) break;
      if (!isVisible(el)) continue;
      if (!el.matches(actionable)) {
        const view = el.ownerDocument.defaultView;
        if (!view || view.getComputedStyle(el).cursor !== "pointer") continue;
        const label = normalize(el.textContent);
        if (!label || label.length > 60) continue;
      }
      const zone = zoneOf(el);
      (zone === "main" ? main : peripheral).push({ el, zone });
    }
    const picked = [...main.slice(0, 120), ...peripheral.slice(0, 30)];
    const elements = picked.map(({ el, zone }, index) => {
      const rect = el.getBoundingClientRect();
      const select = el instanceof HTMLSelectElement ? el : null;
      const name = clip(accessibleName(el), 160);
      return {
        ref: index + 1,
        role: implicitRole(el),
        name: name || undefined,
        tag: tagOf(el),
        zone,
        context: contextText(el),
        type: el instanceof HTMLInputElement ? el.type : undefined,
        text: el.textContent && normalize(el.textContent) !== name ? clip(el.textContent) : undefined,
        href: el instanceof HTMLAnchorElement ? (el.getAttribute("href") || "").slice(0, 200) : undefined,
        options: select ? Array.from(select.options).slice(0, 40).map((option) => ({
          value: option.value,
          label: clip(option.textContent, 80),
        })) : undefined,
        selectors: candidateSelectors(el).slice(0, 2),
        rect: { x: rect.left, y: rect.top, width: rect.width, height: rect.height },
      };
    });
    return {
      url: location.href,
      title: document.title,
      text: (document.body ? document.body.innerText : "").slice(0, 3000),
      elements,
    };
  }

  function call(method, args = {}) {
    switch (method) {
      case "ping":
        return { version: VERSION, url: location.href, title: document.title, readyState: document.readyState };
      case "locate": {
        const result = locate(args.target);
        const el = result.element;
        return {
          status: result.status,
          score: result.score,
          reasons: result.reasons,
          candidates: result.candidates,
          element: el ? { tag: tagOf(el), role: implicitRole(el), name: clip(accessibleName(el), 80) } : undefined,
        };
      }
      case "snapshot":
        return snapshot(args.target);
      case "perform":
        return perform(args);
      case "check":
        return checkCondition(args.condition);
      case "extract":
        return extract(args);
      case "scroll":
        return scroll(args);
      case "observe":
        return observe();
      default:
        throw new Error(`unknown page agent method: ${method}`);
    }
  }

  const global = globalThis;
  if (!global.__doonce || global.__doonce.version < VERSION) {
    global.__doonce = { version: VERSION, call };
  }
})();
