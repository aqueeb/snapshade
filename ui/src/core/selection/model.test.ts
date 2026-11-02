// SelectionModel behaviour (Plan 01-05 Task 1, Tests 3-8). Rules come from
// docs/SPEC.md §3.4-3.8; each `describe` names the rule it pins.
import { describe, expect, it } from "vitest";
import { SelectionModel, type Mods } from "./model";

const BOUNDS = { w: 3024, h: 1964 };
const NONE: Mods = { meta: false, ctrl: false, shift: false, alt: false };
const META: Mods = { ...NONE, meta: true };

function selected(rect = { x: 100, y: 100, w: 200, h: 150 }): SelectionModel {
  const m = new SelectionModel(BOUNDS);
  m.pointerDown({ x: rect.x, y: rect.y }, NONE);
  m.pointerMove({ x: rect.x + rect.w, y: rect.y + rect.h });
  m.pointerUp({ x: rect.x + rect.w, y: rect.y + rect.h });
  return m;
}

describe("drag creates a normalised rect (spec §3.4; anchor-based drag)", () => {
  it("down → move → up yields selected {100,100,200,150}", () => {
    const m = new SelectionModel(BOUNDS);
    expect(m.state.kind).toBe("idle");
    m.pointerDown({ x: 100, y: 100 }, NONE);
    expect(m.state.kind).toBe("dragging");
    m.pointerMove({ x: 300, y: 250 });
    const up = m.pointerUp({ x: 300, y: 250 });
    expect(up.quickCopy).toBe(false);
    expect(m.state.kind).toBe("selected");
    expect(m.rect()).toEqual({ x: 100, y: 100, w: 200, h: 150 });
  });

  it("dragging from the bottom-right anchor normalises to the same rect", () => {
    const m = new SelectionModel(BOUNDS);
    m.pointerDown({ x: 300, y: 250 }, NONE);
    m.pointerMove({ x: 100, y: 100 });
    m.pointerUp({ x: 100, y: 100 });
    expect(m.rect()).toEqual({ x: 100, y: 100, w: 200, h: 150 });
  });

  it("the live rect is clamped to the display while dragging", () => {
    const m = new SelectionModel(BOUNDS);
    m.pointerDown({ x: 3000, y: 1900 }, NONE);
    m.pointerMove({ x: 5000, y: 5000 });
    expect(m.rect()).toEqual({ x: 3000, y: 1900, w: 24, h: 64 });
  });
});

describe("body drag moves, handles resize (8 handles: squares[8], getResizeActionByPoint:)", () => {
  it("pointerDown inside the body moves the rect by the pointer delta", () => {
    const m = selected();
    expect(m.hitTest({ x: 200, y: 175 })).toBe("body");
    m.pointerDown({ x: 200, y: 175 }, NONE);
    expect(m.state.kind).toBe("moving");
    m.pointerMove({ x: 250, y: 195 });
    expect(m.rect()).toEqual({ x: 150, y: 120, w: 200, h: 150 });
    m.pointerUp({ x: 250, y: 195 });
    expect(m.state.kind).toBe("selected");
    expect(m.rect()).toEqual({ x: 150, y: 120, w: 200, h: 150 });
  });

  it("moving clamps at the display bounds", () => {
    const m = selected();
    m.pointerDown({ x: 200, y: 175 }, NONE);
    m.pointerMove({ x: 20000, y: 20000 });
    m.pointerUp({ x: 20000, y: 20000 });
    expect(m.rect()).toEqual({ x: 3024 - 200, y: 1964 - 150, w: 200, h: 150 });
  });

  it("the SE handle grows w/h by the pointer delta", () => {
    const m = selected();
    expect(m.hitTest({ x: 300 + 3, y: 250 - 2 })).toBe("se");
    m.pointerDown({ x: 303, y: 248 }, NONE);
    expect(m.state.kind).toBe("resizing");
    m.pointerMove({ x: 313, y: 258 });
    m.pointerUp({ x: 313, y: 258 });
    expect(m.rect()).toEqual({ x: 100, y: 100, w: 210, h: 160 });
  });

  it("the NW handle moves x/y and grows w/h", () => {
    const m = selected();
    expect(m.hitTest({ x: 100, y: 100 })).toBe("nw");
    m.pointerDown({ x: 100, y: 100 }, NONE);
    m.pointerMove({ x: 95, y: 95 });
    m.pointerUp({ x: 95, y: 95 });
    expect(m.rect()).toEqual({ x: 95, y: 95, w: 205, h: 155 });
  });

  it("edge handles move one edge only", () => {
    const m = selected();
    expect(m.hitTest({ x: 300, y: 175 })).toBe("e");
    m.pointerDown({ x: 300, y: 175 }, NONE);
    m.pointerMove({ x: 320, y: 300 });
    m.pointerUp({ x: 320, y: 300 });
    expect(m.rect()).toEqual({ x: 100, y: 100, w: 220, h: 150 });
    expect(m.hitTest({ x: 210, y: 100 })).toBe("n");
  });

  it("a resize past the opposite edge keeps min 1x1", () => {
    const m = selected();
    m.pointerDown({ x: 300, y: 250 }, NONE); // se
    m.pointerMove({ x: 0, y: 0 });
    m.pointerUp({ x: 0, y: 0 });
    expect(m.rect()).toEqual({ x: 100, y: 100, w: 1, h: 1 });
  });
});

describe("keyboard: arrows nudge 1 px, Shift+arrows resize 1 px on the right/bottom edge (spec §3.4; moveSelection:withFlags:fast:)", () => {
  it("nudge(1,0) moves x by exactly 1 and clamps at bounds.w - rect.w", () => {
    const m = selected();
    m.nudge(1, 0);
    expect(m.rect()).toEqual({ x: 101, y: 100, w: 200, h: 150 });
    m.nudge(0, -1);
    expect(m.rect()).toEqual({ x: 101, y: 99, w: 200, h: 150 });
    for (let i = 0; i < 4000; i += 1) m.nudge(1, 0);
    expect(m.rect()!.x).toBe(3024 - 200);
    for (let i = 0; i < 4000; i += 1) m.nudge(0, -1);
    expect(m.rect()!.y).toBe(0);
  });

  it("resizeBy(1,0) grows w by 1 and clamps at the bounds", () => {
    const m = selected();
    m.resizeBy(1, 0);
    expect(m.rect()).toEqual({ x: 100, y: 100, w: 201, h: 150 });
    m.resizeBy(0, 1);
    expect(m.rect()).toEqual({ x: 100, y: 100, w: 201, h: 151 });
    for (let i = 0; i < 4000; i += 1) m.resizeBy(1, 1);
    expect(m.rect()).toEqual({ x: 100, y: 100, w: 3024 - 100, h: 1964 - 100 });
  });

  it("resizeBy(-1,0) on w=1 stays 1", () => {
    const m = selected({ x: 10, y: 10, w: 1, h: 1 });
    expect(m.rect()).toEqual({ x: 10, y: 10, w: 1, h: 1 });
    m.resizeBy(-1, -1);
    expect(m.rect()).toEqual({ x: 10, y: 10, w: 1, h: 1 });
  });

  it("nudge and resizeBy are no-ops while idle", () => {
    const m = new SelectionModel(BOUNDS);
    m.nudge(1, 1);
    m.resizeBy(1, 1);
    expect(m.state.kind).toBe("idle");
    expect(m.rect()).toBeNull();
  });
});

describe("⌘A toggles full display and restores the previous rect (spec §3.8)", () => {
  it("from rect R yields the full display, then restores R", () => {
    const m = selected();
    m.toggleFullscreen();
    expect(m.rect()).toEqual({ x: 0, y: 0, w: 3024, h: 1964 });
    m.toggleFullscreen();
    expect(m.rect()).toEqual({ x: 100, y: 100, w: 200, h: 150 });
  });

  it("from idle it selects the full display", () => {
    const m = new SelectionModel(BOUNDS);
    m.toggleFullscreen();
    expect(m.state.kind).toBe("selected");
    expect(m.rect()).toEqual({ x: 0, y: 0, w: 3024, h: 1964 });
  });
});

describe("click-outside restarts; ⌘/Ctrl at pointerDown flags quick copy (spec §3.4, §3.6; startNewSelection)", () => {
  it("pointerDown outside the selected rect starts a new dragging state", () => {
    const m = selected();
    expect(m.hitTest({ x: 1000, y: 1000 })).toBe("outside");
    m.pointerDown({ x: 1000, y: 1000 }, NONE);
    expect(m.state.kind).toBe("dragging");
    m.pointerMove({ x: 1010, y: 1020 });
    m.pointerUp({ x: 1010, y: 1020 });
    expect(m.rect()).toEqual({ x: 1000, y: 1000, w: 10, h: 20 });
  });

  it("pointerDown with meta sets quickCopy and pointerUp reports it", () => {
    const m = new SelectionModel(BOUNDS);
    m.pointerDown({ x: 10, y: 10 }, META);
    expect(m.state).toMatchObject({ kind: "dragging", quickCopy: true });
    m.pointerMove({ x: 50, y: 50 });
    expect(m.pointerUp({ x: 50, y: 50 })).toEqual({ quickCopy: true });
  });

  it("ctrl behaves like meta (Windows Ctrl-drag)", () => {
    const m = new SelectionModel(BOUNDS);
    m.pointerDown({ x: 10, y: 10 }, { ...NONE, ctrl: true });
    m.pointerMove({ x: 50, y: 50 });
    expect(m.pointerUp({ x: 50, y: 50 })).toEqual({ quickCopy: true });
  });

  it("a move or resize pointerUp never reports quickCopy", () => {
    const m = selected();
    m.pointerDown({ x: 200, y: 175 }, META);
    expect(m.pointerUp({ x: 200, y: 175 })).toEqual({ quickCopy: false });
  });
});

describe("clear and hitTest while idle", () => {
  it("clear() returns to idle", () => {
    const m = selected();
    m.clear();
    expect(m.state.kind).toBe("idle");
    expect(m.rect()).toBeNull();
  });

  it("hitTest returns 'outside' when idle", () => {
    const m = new SelectionModel(BOUNDS);
    expect(m.hitTest({ x: 0, y: 0 })).toBe("outside");
  });

  it("a zero-area drag (a click) leaves no selection", () => {
    const m = new SelectionModel(BOUNDS);
    m.pointerDown({ x: 10, y: 10 }, NONE);
    m.pointerUp({ x: 10, y: 10 });
    expect(m.state.kind).toBe("idle");
  });
});
