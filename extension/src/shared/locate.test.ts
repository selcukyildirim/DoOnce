import { describe, expect, it } from "vitest";
import { locate } from "../page-agent/locate";

describe("locate", () => {
  it("finds a button by role and name", () => {
    document.body.innerHTML = `<button id="save-contact" class="primary">Save contact</button>`;
    const r = locate({ role: "button", name: "Save contact", selectors: ["#save-contact"] });
    expect(r.status).toBe("found");
  });
});
