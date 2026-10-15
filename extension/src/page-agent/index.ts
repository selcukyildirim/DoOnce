// Entry point of page-agent.js. The same bundle is injected by the browser
// extension (chrome.scripting) and by the Rust CDP driver
// (Runtime.evaluate in an isolated world), so replay behaves identically.

import { accessibleName, implicitRole, tagOf } from "../shared/dom";
import type { Condition, Target } from "../shared/types";
import { checkCondition, perform, type PerformArgs } from "./actions";
import { locate } from "./locate";

export const AGENT_VERSION = 1;

type Args = Record<string, unknown>;

function call(method: string, args: Args = {}): unknown {
  switch (method) {
    case "ping":
      return { version: AGENT_VERSION, url: location.href, readyState: document.readyState };
    case "locate": {
      const r = locate(args.target as Target);
      const el = r.element;
      return {
        status: r.status,
        score: r.score,
        reasons: r.reasons,
        candidates: r.candidates,
        element: el ? { tag: tagOf(el), role: implicitRole(el), name: accessibleName(el).slice(0, 80) } : undefined,
      };
    }
    case "perform":
      return perform(args as unknown as PerformArgs);
    case "check":
      return checkCondition(args.condition as Condition);
    default:
      throw new Error(`unknown page agent method: ${method}`);
  }
}

const g = globalThis as { __doonce?: { version: number; call: typeof call } };
if (!g.__doonce || g.__doonce.version < AGENT_VERSION) {
  g.__doonce = { version: AGENT_VERSION, call };
}
