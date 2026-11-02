import { describe, expect, it } from "vitest";
import { layoutBadge, layoutToolbars, physicalToCss } from "./layout";

const bounds = { w: 1000, h: 600 };
const tools = { w: 28, h: 280 };
const actions = { w: 120, h: 28 };
const gap = 4;

describe("layoutToolbars (spec §3.5; ANNOT-14)", () => {
  it("default: tool strip right of the selection, top-aligned; action strip below, right-aligned", () => {
    const l = layoutToolbars({ sel: { x: 100, y: 100, w: 300, h: 200 }, bounds, tools, actions, gap });
    expect(l.tools).toEqual({ x: 404, y: 100, side: "right" });
    expect(l.actions).toEqual({ x: 280, y: 304, side: "below" });
  });

  it("flips the tool strip to the left when the right edge has no room", () => {
    const l = layoutToolbars({ sel: { x: 680, y: 100, w: 300, h: 200 }, bounds, tools, actions, gap });
    expect(l.tools).toEqual({ x: 648, y: 100, side: "left" });
  });

  it("puts the tool strip inside when neither side fits (near-full-width selection)", () => {
    const l = layoutToolbars({ sel: { x: 10, y: 100, w: 980, h: 200 }, bounds, tools, actions, gap });
    expect(l.tools.side).toBe("inside");
    expect(l.tools.x).toBe(990 - 4 - 28);
  });

  it("flips the action strip above when the bottom has no room, inside when neither fits", () => {
    const above = layoutToolbars({ sel: { x: 100, y: 300, w: 300, h: 290 }, bounds, tools, actions, gap });
    expect(above.actions).toEqual({ x: 280, y: 268, side: "above" });
    const inside = layoutToolbars({ sel: { x: 0, y: 0, w: 1000, h: 600 }, bounds, tools, actions, gap });
    expect(inside.actions.side).toBe("inside");
    expect(inside.tools.side).toBe("inside");
    // Both inside: the action strip must not sit on top of the tool strip.
    expect(inside.actions.y >= inside.tools.y + tools.h || inside.actions.x + actions.w <= inside.tools.x).toBe(true);
  });

  it("clamps the tool strip vertically and the action strip horizontally into the display", () => {
    const l = layoutToolbars({ sel: { x: 10, y: 500, w: 50, h: 90 }, bounds, tools, actions, gap });
    expect(l.tools.y).toBe(600 - 280);
    expect(l.actions.x).toBe(0);
  });

  it("physicalToCss divides by the scale", () => {
    expect(physicalToCss({ x: 200, y: 100, w: 50, h: 30 }, 2)).toEqual({ x: 100, y: 50, w: 25, h: 15 });
  });
});

describe("layoutBadge", () => {
  it("sits above the top-left corner, moves below when touching the top, clamps to the display", () => {
    expect(layoutBadge({ x: 100, y: 100, w: 50, h: 50 }, bounds, { w: 60, h: 18 }, 4)).toEqual({ x: 100, y: 78 });
    expect(layoutBadge({ x: 100, y: 5, w: 50, h: 50 }, bounds, { w: 60, h: 18 }, 4)).toEqual({ x: 100, y: 9 });
    expect(layoutBadge({ x: 980, y: 100, w: 20, h: 50 }, bounds, { w: 60, h: 18 }, 4)).toEqual({ x: 940, y: 78 });
  });
});
