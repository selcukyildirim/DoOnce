// Finds the element a Target describes by scoring every plausible candidate
// against all recorded signals. Deterministic: same DOM, same answer.

import {
  accessibleName, contextText, implicitRole, isDisabled, isVisible, labelText, normalize, roleQuery, tagOf,
} from "../shared/dom";
import type { Target } from "../shared/types";

export type LocateStatus = "found" | "notFound" | "ambiguous" | "hidden" | "disabled";

export interface Candidate {
  element: Element;
  score: number;
  reasons: string[];
}

export interface LocateResult {
  status: LocateStatus;
  element?: Element;
  score?: number;
  /** Which signals matched, for diagnostics and self-healing. */
  reasons?: string[];
  /** Best candidates, summarized without field values. */
  candidates?: { score: number; tag: string; role?: string; name?: string; context?: string }[];
}

/** Minimum score to accept a match. */
export const ACCEPT_SCORE = 45;
/** The best match must beat the runner-up by this much. */
export const MARGIN = 15;
const MAX_CANDIDATES = 3000;

function eq(a: string | undefined, b: string | undefined): boolean {
  return !!a && !!b && normalize(a).toLowerCase() === normalize(b).toLowerCase();
}

function tokens(s: string): Set<string> {
  return new Set(normalize(s).toLowerCase().split(/[^\p{L}\p{N}]+/u).filter(Boolean));
}

function overlap(a: string, b: string): number {
  const ta = tokens(a);
  const tb = tokens(b);
  if (!ta.size || !tb.size) return 0;
  let common = 0;
  ta.forEach((t) => { if (tb.has(t)) common++; });
  return common / Math.max(ta.size, tb.size);
}

function query(root: Document, selector: string): Element[] {
  try {
    return Array.from(root.querySelectorAll(selector));
  } catch {
    return [];
  }
}

function gatherCandidates(doc: Document, target: Target, selectorHits: Map<Element, number>): Set<Element> {
  const out = new Set<Element>();
  (target.selectors ?? []).forEach((sel, index) => {
    const matches = query(doc, sel);
    for (const el of matches) {
      out.add(el);
      // Unique selector matches are strong evidence, the first one most of all.
      const strength = matches.length === 1 ? (index === 0 ? 40 : 30) : 8;
      selectorHits.set(el, Math.max(selectorHits.get(el) ?? 0, strength));
    }
  });
  const attrs = target.attributes ?? {};
  if (attrs.id) {
    const byId = doc.getElementById(attrs.id);
    if (byId) out.add(byId);
  }
  if (target.role) query(doc, roleQuery(target.role)).forEach((el) => out.add(el));
  if (target.tag && out.size < MAX_CANDIDATES) {
    query(doc, target.tag).slice(0, MAX_CANDIDATES).forEach((el) => out.add(el));
  }
  if (target.label) query(doc, "input, select, textarea").forEach((el) => out.add(el));
  return out;
}

export function scoreElement(el: Element, target: Target, selectorScore: number): Candidate {
  let score = selectorScore;
  const reasons: string[] = selectorScore ? [`selector+${selectorScore}`] : [];
  const add = (points: number, reason: string) => {
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
    if (eq(label, target.label)) add(20, "label");
    else if (label && overlap(label, target.label) >= 0.6) add(10, "label~");
  }
  if (target.text && eq(normalize(el.textContent), target.text)) add(10, "text");

  const attrs = target.attributes ?? {};
  const weights: Record<string, number> = {
    "data-testid": 30, "data-test": 30, "data-qa": 30, "data-cy": 30, id: 25, name: 15, "aria-label": 10,
    placeholder: 10, href: 10, type: 5, "data-action": 10, title: 5,
  };
  for (const [key, weight] of Object.entries(weights)) {
    const want = attrs[key];
    if (!want) continue;
    const have = el.getAttribute(key);
    if (have === want) add(weight, key);
    else if (have && (key === "id" || key.startsWith("data-t") || key === "name")) add(-10, key);
  }
  // Context is the most expensive signal; skip it for hopeless candidates.
  if (target.context && score >= 20) {
    const context = contextText(el);
    if (eq(context, target.context)) add(15, "context");
    else if (context) add(-15, "context");
  }
  return { element: el, score, reasons };
}

function summarize(c: Candidate) {
  return {
    score: c.score,
    tag: tagOf(c.element),
    role: implicitRole(c.element),
    name: accessibleName(c.element).slice(0, 80) || undefined,
    context: contextText(c.element),
  };
}

export function locate(target: Target, doc: Document = document): LocateResult {
  const selectorHits = new Map<Element, number>();
  const pool = gatherCandidates(doc, target, selectorHits);
  const scored: Candidate[] = [];
  pool.forEach((el) => scored.push(scoreElement(el, target, selectorHits.get(el) ?? 0)));
  scored.sort((a, b) => b.score - a.score);

  const visible = scored.filter((c) => isVisible(c.element));
  const best = visible[0];
  const top = visible.slice(0, 3).map(summarize);
  if (!best || best.score < ACCEPT_SCORE) {
    // Distinguish "it is there but hidden" from "it is gone".
    const hidden = scored[0];
    if (hidden && hidden.score >= ACCEPT_SCORE && !isVisible(hidden.element)) {
      return { status: "hidden", score: hidden.score, reasons: hidden.reasons, candidates: top };
    }
    return { status: "notFound", score: best?.score, candidates: top };
  }
  const second = visible[1];
  if (second && best.score - second.score < MARGIN) {
    return { status: "ambiguous", score: best.score, reasons: best.reasons, candidates: top };
  }
  if (isDisabled(best.element)) {
    return { status: "disabled", element: best.element, score: best.score, reasons: best.reasons };
  }
  return { status: "found", element: best.element, score: best.score, reasons: best.reasons };
}
