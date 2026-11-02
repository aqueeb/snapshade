// CaptureSession (Plan 01-05 Task 3, Tests 1-7): OverlayInput → SessionEvent[]
// as a pure transition (ARCHITECTURE Pattern 4). Keys follow docs/SPEC.md §3.6-3.8,
// §4 (wheel), §5 (action hotkeys) and the KEYMAP table in session.ts.
import { describe, expect, it } from "vitest";
import { makeFrame } from "../../test/fixture-frame";
import { CaptureSession, KEYMAP, type OverlayInput, type SessionEvent } from "./session";
import { newTopology, physicalSizeFor, type DisplayInfo, type DisplayTopology } from "./geometry";
import type { Mods } from "./selection/model";

const NONE: Mods = { meta: false, ctrl: false, shift: false, alt: false };
const META: Mods = { ...NONE, meta: true };
const CTRL: Mods = { ...NONE, ctrl: true };
const SHIFT: Mods = { ...NONE, shift: true };

// fixtures/geometry/displays.json layout "retina-plus-1x-right"
function retinaPlus1x(): DisplayTopology {
  const d = (id: number, l: [number, number, number, number], scale: number, primary: boolean): DisplayInfo => {
    const logical = { x: l[0], y: l[1], w: l[2], h: l[3] };
    return { id, logical, scale, physical: physicalSizeFor(logical, scale), primary };
  };
  return newTopology([d(2, [1512, -229, 2560, 1440], 1, false), d(1, [0, 0, 1512, 982], 2, true)]);
}

function session(): CaptureSession {
  const frames = new Map([
    [1, makeFrame(8, 8)],
    [2, makeFrame(8, 8)],
  ]);
  return new CaptureSession(retinaPlus1x(), frames);
}

function drag(s: CaptureSession, display: number, a: [number, number], b: [number, number], mods: Mods = NONE): SessionEvent[][] {
  return [
    s.handle({ t: "pointerDown", display, p: { x: a[0], y: a[1] }, mods }),
    s.handle({ t: "pointerMove", display, p: { x: b[0], y: b[1] } }),
    s.handle({ t: "pointerUp", display, p: { x: b[0], y: b[1] } }),
  ];
}

function key(s: CaptureSession, k: string, mods: Mods = NONE): SessionEvent[] {
  return s.handle({ t: "key", key: k, mods });
}

describe("home display (ARCHITECTURE Pattern 2: a selection never spans displays)", () => {
  it("pointerDown on display 2 sets homeDisplay to 2 with bounds = that display's physical size", () => {
    const s = session();
    expect(s.homeDisplay).toBeNull();
    expect(s.selection()).toBeNull();
    s.handle({ t: "pointerDown", display: 2, p: { x: 10, y: 10 }, mods: NONE });
    expect(s.homeDisplay).toBe(2);
    expect(s.selection()?.model.bounds).toEqual({ w: 2560, h: 1440 });
  });

  it("a later pointerDown on display 1 while idle re-homes to 1", () => {
    const s = session();
    s.handle({ t: "pointerDown", display: 2, p: { x: 10, y: 10 }, mods: NONE });
    s.handle({ t: "pointerUp", display: 2, p: { x: 10, y: 10 } }); // zero-area click → idle
    s.handle({ t: "pointerDown", display: 1, p: { x: 5, y: 5 }, mods: NONE });
    expect(s.homeDisplay).toBe(1);
    expect(s.selection()?.model.bounds).toEqual({ w: 3024, h: 1964 });
  });

  it("starting a drag on another display replaces the selection and its annotations (hideAllSelections)", () => {
    const s = session();
    drag(s, 2, [10, 10], [110, 60]);
    s.handle({ t: "tool", tool: "pen" });
    drag(s, 2, [20, 20], [40, 40]);
    expect(s.doc.shapes.length).toBe(1);
    s.handle({ t: "tool", tool: null });
    drag(s, 1, [0, 0], [100, 100]);
    expect(s.homeDisplay).toBe(1);
    expect(s.selection()?.model.rect()).toEqual({ x: 0, y: 0, w: 100, h: 100 });
    expect(s.doc.shapes.length).toBe(0);
  });

  it("pointer input for an unknown display is ignored", () => {
    const s = session();
    expect(s.handle({ t: "pointerDown", display: 9, p: { x: 1, y: 1 }, mods: NONE })).toEqual([]);
    expect(s.homeDisplay).toBeNull();
  });
});

describe("drag emits sceneChanged; ⌘-drag quick-copies (spec §3.4, §3.6)", () => {
  it("down/move/up each emit [{sceneChanged, display}] and leave a selection", () => {
    const s = session();
    const out = drag(s, 2, [10, 10], [110, 60]);
    expect(out).toEqual([
      [{ t: "sceneChanged", display: 2 }],
      [{ t: "sceneChanged", display: 2 }],
      [{ t: "sceneChanged", display: 2 }],
    ]);
    expect(s.selection()?.model.rect()).toEqual({ x: 10, y: 10, w: 100, h: 50 });
  });

  it("with meta at pointerDown, pointerUp emits export clipboard then dismiss", () => {
    const s = session();
    const out = drag(s, 2, [10, 10], [110, 60], META);
    expect(out[2]).toEqual([{ t: "export", kind: "clipboard" }, { t: "dismiss" }]);
  });

  it("ctrl at pointerDown behaves like meta (Windows Ctrl-drag)", () => {
    const s = session();
    const out = drag(s, 1, [10, 10], [110, 60], CTRL);
    expect(out[2]).toEqual([{ t: "export", kind: "clipboard" }, { t: "dismiss" }]);
  });

  it("pointerMove without a prior pointerDown emits nothing", () => {
    const s = session();
    expect(s.handle({ t: "pointerMove", display: 1, p: { x: 1, y: 1 } })).toEqual([]);
  });
});

describe("keys: Esc / ⌘X dismiss, ⌘A full display, arrows nudge, Shift+arrows resize, ⌘Z undo", () => {
  it("Escape emits dismiss (spec §3.7)", () => {
    const s = session();
    drag(s, 2, [10, 10], [110, 60]);
    expect(key(s, "Escape")).toEqual([{ t: "dismiss" }]);
  });

  it("Escape before any selection also dismisses the overlay", () => {
    expect(key(session(), "Escape")).toEqual([{ t: "dismiss" }]);
  });

  it("⌘X and Ctrl+X emit dismiss (spec §3.7)", () => {
    const s = session();
    drag(s, 2, [10, 10], [110, 60]);
    expect(key(s, "x", META)).toEqual([{ t: "dismiss" }]);
    expect(key(s, "x", CTRL)).toEqual([{ t: "dismiss" }]);
    expect(key(s, "x")).toEqual([]); // plain x is nothing
  });

  it("⌘A selects the full home display and emits sceneChanged; again restores (spec §3.8)", () => {
    const s = session();
    drag(s, 2, [10, 10], [110, 60]);
    expect(key(s, "a", META)).toEqual([{ t: "sceneChanged", display: 2 }]);
    expect(s.selection()?.model.rect()).toEqual({ x: 0, y: 0, w: 2560, h: 1440 });
    key(s, "a", META);
    expect(s.selection()?.model.rect()).toEqual({ x: 10, y: 10, w: 100, h: 50 });
  });

  it("arrows nudge 1 px; Shift+arrows resize 1 px on the right/bottom edge (spec §3.4)", () => {
    const s = session();
    drag(s, 2, [10, 10], [110, 60]);
    expect(key(s, "ArrowRight")).toEqual([{ t: "sceneChanged", display: 2 }]);
    expect(s.selection()?.model.rect()).toEqual({ x: 11, y: 10, w: 100, h: 50 });
    key(s, "ArrowDown");
    key(s, "ArrowLeft");
    key(s, "ArrowUp");
    expect(s.selection()?.model.rect()).toEqual({ x: 10, y: 10, w: 100, h: 50 });
    expect(key(s, "ArrowRight", SHIFT)).toEqual([{ t: "sceneChanged", display: 2 }]);
    expect(s.selection()?.model.rect()).toEqual({ x: 10, y: 10, w: 101, h: 50 });
    key(s, "ArrowDown", SHIFT);
    key(s, "ArrowLeft", SHIFT);
    key(s, "ArrowUp", SHIFT);
    expect(s.selection()?.model.rect()).toEqual({ x: 10, y: 10, w: 100, h: 50 });
  });

  it("⌘Z undoes the last annotation and emits sceneChanged; nothing to undo emits [] (ANNOT-11)", () => {
    const s = session();
    drag(s, 2, [10, 10], [110, 60]);
    s.handle({ t: "tool", tool: "pen" });
    drag(s, 2, [20, 20], [40, 40]);
    expect(s.doc.shapes.length).toBe(1);
    expect(key(s, "z", META)).toEqual([{ t: "sceneChanged", display: 2 }]);
    expect(s.doc.shapes.length).toBe(0);
    expect(key(s, "z", META)).toEqual([]);
    expect(key(s, "z", CTRL)).toEqual([]);
  });
});

describe("export keys and actions (spec §5; events only — exporters are Phase 3)", () => {
  it("⌘C / ⌘S / ⌘P emit export clipboard / save / print", () => {
    const s = session();
    drag(s, 2, [10, 10], [110, 60]);
    expect(key(s, "c", META)).toEqual([{ t: "export", kind: "clipboard" }]);
    expect(key(s, "s", META)).toEqual([{ t: "export", kind: "save" }]);
    expect(key(s, "p", META)).toEqual([{ t: "export", kind: "print" }]);
    expect(key(s, "c", CTRL)).toEqual([{ t: "export", kind: "clipboard" }]);
  });

  it("actions copy/save/print emit export; close emits dismiss; undo undoes", () => {
    const s = session();
    drag(s, 2, [10, 10], [110, 60]);
    expect(s.handle({ t: "action", action: "copy" })).toEqual([{ t: "export", kind: "clipboard" }]);
    expect(s.handle({ t: "action", action: "save" })).toEqual([{ t: "export", kind: "save" }]);
    expect(s.handle({ t: "action", action: "print" })).toEqual([{ t: "export", kind: "print" }]);
    expect(s.handle({ t: "action", action: "close" })).toEqual([{ t: "dismiss" }]);
    s.handle({ t: "tool", tool: "rect" });
    drag(s, 2, [20, 20], [40, 40]);
    expect(s.doc.shapes.length).toBe(1);
    expect(s.handle({ t: "action", action: "undo" })).toEqual([{ t: "sceneChanged", display: 2 }]);
    expect(s.doc.shapes.length).toBe(0);
  });
});

describe("tools: tool-first hit testing (ARCHITECTURE Pattern 4; ANNOT-12)", () => {
  it("a drag inside the selection with the pen active commits a pen shape and does not move the selection", () => {
    const s = session();
    drag(s, 2, [10, 10], [110, 60]);
    s.handle({ t: "tool", tool: "pen" });
    const out = drag(s, 2, [30, 30], [50, 40]);
    expect(out[2]).toEqual([{ t: "sceneChanged", display: 2 }]);
    expect(s.selection()?.model.rect()).toEqual({ x: 10, y: 10, w: 100, h: 50 });
    expect(s.doc.shapes).toEqual([
      { kind: "pen", pts: [{ x: 20, y: 20 }, { x: 40, y: 30 }], color: "#ff0000", size: 3 },
    ]);
  });

  it("shape coordinates are selection-local; rect/ellipse/blur normalise the drag; marker uses the marker colour", () => {
    const s = session();
    drag(s, 2, [10, 10], [110, 60]);
    s.handle({ t: "tool", tool: "rect" });
    drag(s, 2, [60, 50], [30, 30]);
    s.handle({ t: "tool", tool: "marker" });
    drag(s, 2, [30, 30], [50, 30]);
    s.handle({ t: "tool", tool: "arrow" });
    drag(s, 2, [30, 30], [50, 40]);
    expect(s.doc.shapes).toEqual([
      { kind: "rect", r: { x: 20, y: 20, w: 30, h: 20 }, color: "#ff0000", size: 3 },
      { kind: "marker", pts: [{ x: 20, y: 20 }, { x: 40, y: 20 }], color: "#ffff00", size: 3 },
      { kind: "arrow", a: { x: 20, y: 20 }, b: { x: 40, y: 30 }, color: "#ff0000", size: 3 },
    ]);
  });

  it("a drag starting on a handle resizes even with a tool active (ANNOT-12)", () => {
    const s = session();
    drag(s, 2, [10, 10], [110, 60]);
    s.handle({ t: "tool", tool: "rect" });
    drag(s, 2, [110, 60], [120, 70]); // SE corner
    expect(s.selection()?.model.rect()).toEqual({ x: 10, y: 10, w: 110, h: 60 });
    expect(s.doc.shapes.length).toBe(0);
  });

  it("a drag outside the selection with a tool active starts a new selection", () => {
    const s = session();
    drag(s, 2, [10, 10], [110, 60]);
    s.handle({ t: "tool", tool: "pen" });
    drag(s, 2, [500, 500], [600, 600]);
    expect(s.selection()?.model.rect()).toEqual({ x: 500, y: 500, w: 100, h: 100 });
    expect(s.doc.shapes.length).toBe(0);
  });

  it("tool null returns to selection mode: a drag inside moves the selection again", () => {
    const s = session();
    drag(s, 2, [10, 10], [110, 60]);
    s.handle({ t: "tool", tool: "pen" });
    s.handle({ t: "tool", tool: null });
    drag(s, 2, [50, 30], [60, 40]);
    expect(s.selection()?.model.rect()).toEqual({ x: 20, y: 20, w: 100, h: 50 });
    expect(s.doc.shapes.length).toBe(0);
  });
});

describe("wheel changes the tool size (spec §4: wheel down = bigger, clamped)", () => {
  it("deltaY > 0 increases size by 1, < 0 decreases, clamped to [min, max]", () => {
    const s = session();
    drag(s, 2, [10, 10], [110, 60]);
    s.handle({ t: "tool", tool: "pen" });
    expect(s.handle({ t: "wheel", deltaY: 120 })).toEqual([{ t: "sceneChanged", display: 2 }]);
    expect(s.tool.size).toBe(4);
    s.handle({ t: "wheel", deltaY: -1 });
    expect(s.tool.size).toBe(3);
    for (let i = 0; i < 100; i += 1) s.handle({ t: "wheel", deltaY: 1 });
    expect(s.tool.size).toBe(s.tool.max);
    expect(s.handle({ t: "wheel", deltaY: 1 })).toEqual([]); // already clamped: no change
    for (let i = 0; i < 100; i += 1) s.handle({ t: "wheel", deltaY: -1 });
    expect(s.tool.size).toBe(s.tool.min);
  });

  it("the wheel is ignored while no tool is active (spec §4 'while a tool is active')", () => {
    const s = session();
    drag(s, 2, [10, 10], [110, 60]);
    expect(s.handle({ t: "wheel", deltaY: 120 })).toEqual([]);
    expect(s.tool.size).toBe(3);
  });
});

describe("guards", () => {
  it("selection-dependent keys before a selection exists are ignored, never thrown", () => {
    const s = session();
    expect(key(s, "ArrowRight")).toEqual([]);
    expect(key(s, "c", META)).toEqual([]);
    expect(key(s, "z", META)).toEqual([]);
    expect(s.handle({ t: "action", action: "copy" })).toEqual([]);
    expect(s.handle({ t: "action", action: "undo" })).toEqual([]);
    expect(s.selection()).toBeNull();
  });

  it("⌘A with no selection selects the whole display the key arrived on (SELECT-05); without a display it is ignored", () => {
    const s = session();
    expect(key(s, "a", META)).toEqual([]);
    expect(s.handle({ t: "key", key: "a", mods: META, display: 2 })).toEqual([{ t: "sceneChanged", display: 2 }]);
    expect(s.homeDisplay).toBe(2);
    expect(s.selection()?.model.rect()).toEqual({ x: 0, y: 0, w: 2560, h: 1440 });
  });

  it("export keys during a drag (no committed rect) are ignored", () => {
    const s = session();
    s.handle({ t: "pointerDown", display: 2, p: { x: 10, y: 10 }, mods: NONE });
    s.handle({ t: "pointerMove", display: 2, p: { x: 50, y: 50 } });
    expect(key(s, "c", META)).toEqual([]);
  });

  it("unknown keys return []", () => {
    const s = session();
    drag(s, 2, [10, 10], [110, 60]);
    expect(key(s, "F13")).toEqual([]);
    expect(key(s, "q", META)).toEqual([]);
    const weird: OverlayInput = { t: "key", key: "Dead", mods: NONE };
    expect(s.handle(weird)).toEqual([]);
  });

  it("KEYMAP documents every rule with a source", () => {
    expect(KEYMAP.length).toBeGreaterThanOrEqual(10);
    for (const row of KEYMAP) {
      expect(row.source.length).toBeGreaterThan(0);
      expect(row.action.length).toBeGreaterThan(0);
    }
    expect(KEYMAP.some((r) => r.key === "Escape")).toBe(true);
  });
});

describe("Phase 2/3 additions: fullscreen/clear actions, text tool, pending shape, size keys, restore", () => {
  it("action fullscreen toggles like ⌘A and works from idle with a display", () => {
    const s = session();
    expect(s.handle({ t: "action", action: "fullscreen", display: 1 })).toEqual([{ t: "sceneChanged", display: 1 }]);
    expect(s.selection()?.model.rect()).toEqual({ x: 0, y: 0, w: 3024, h: 1964 });
    // From idle there is nothing to restore: a second toggle keeps the full display.
    s.handle({ t: "action", action: "fullscreen" });
    expect(s.selection()?.model.rect()).toEqual({ x: 0, y: 0, w: 3024, h: 1964 });
    s.handle({ t: "action", action: "clear" });
    drag(s, 1, [10, 10], [110, 60]);
    s.handle({ t: "action", action: "fullscreen" });
    expect(s.selection()?.model.rect()).toEqual({ x: 0, y: 0, w: 3024, h: 1964 });
    s.handle({ t: "action", action: "fullscreen" });
    expect(s.selection()?.model.rect()).toEqual({ x: 10, y: 10, w: 100, h: 50 });
  });

  it("action clear drops the rect and the annotations", () => {
    const s = session();
    drag(s, 2, [10, 10], [110, 60]);
    s.handle({ t: "tool", tool: "pen" });
    drag(s, 2, [20, 20], [40, 40]);
    expect(s.handle({ t: "action", action: "clear" })).toEqual([{ t: "sceneChanged", display: 2 }]);
    expect(s.selection()?.model.rect()).toBeNull();
    expect(s.doc.shapes.length).toBe(0);
  });

  it("text tool: a click inside the selection emits beginText at a selection-local point; commitText adds the shape (ANNOT-08)", () => {
    const s = session();
    drag(s, 2, [10, 10], [110, 60]);
    s.handle({ t: "tool", tool: "text" });
    expect(s.handle({ t: "pointerDown", display: 2, p: { x: 30, y: 25 }, mods: NONE })).toEqual([
      { t: "beginText", display: 2, at: { x: 20, y: 15 } },
    ]);
    expect(s.handle({ t: "pointerUp", display: 2, p: { x: 30, y: 25 } })).toEqual([]);
    expect(s.handle({ t: "commitText", at: { x: 20, y: 15 }, text: "héllo 日本" })).toEqual([{ t: "sceneChanged", display: 2 }]);
    expect(s.doc.shapes).toEqual([{ kind: "text", at: { x: 20, y: 15 }, text: "héllo 日本", color: "#ff0000", size: 3 }]);
    // Empty text commits nothing but still repaints.
    expect(s.handle({ t: "commitText", at: { x: 1, y: 1 }, text: "   " })).toEqual([{ t: "sceneChanged", display: 2 }]);
    expect(s.doc.shapes.length).toBe(1);
  });

  it("pendingShape() exposes the stroke in progress and is null otherwise", () => {
    const s = session();
    drag(s, 2, [10, 10], [110, 60]);
    expect(s.pendingShape()).toBeNull();
    s.handle({ t: "tool", tool: "rect" });
    s.handle({ t: "pointerDown", display: 2, p: { x: 20, y: 20 }, mods: NONE });
    s.handle({ t: "pointerMove", display: 2, p: { x: 60, y: 40 } });
    expect(s.pendingShape()).toEqual({ kind: "rect", r: { x: 10, y: 10, w: 40, h: 20 }, color: "#ff0000", size: 3 });
    s.handle({ t: "pointerUp", display: 2, p: { x: 60, y: 40 } });
    expect(s.pendingShape()).toBeNull();
    expect(s.doc.shapes.length).toBe(1);
  });

  it("Ctrl/⌘ + and − change the size like the wheel, only while a tool is active (ANNOT-10)", () => {
    const s = session();
    drag(s, 2, [10, 10], [110, 60]);
    expect(key(s, "=", META)).toEqual([]);
    expect(s.tool.size).toBe(3);
    s.handle({ t: "tool", tool: "pen" });
    expect(key(s, "=", CTRL)).toEqual([{ t: "sceneChanged", display: 2 }]);
    expect(s.tool.size).toBe(4);
    expect(key(s, "+", META)).toEqual([{ t: "sceneChanged", display: 2 }]);
    expect(key(s, "-", META)).toEqual([{ t: "sceneChanged", display: 2 }]);
    expect(s.tool.size).toBe(4);
  });

  it("restore pre-selects a clamped rect on a display (SELECT-08) and saveAs exports", () => {
    const s = session();
    expect(s.handle({ t: "restore", display: 2, rect: { x: 2500, y: 1400, w: 200, h: 100 } })).toEqual([{ t: "sceneChanged", display: 2 }]);
    expect(s.selection()?.model.rect()).toEqual({ x: 2360, y: 1340, w: 200, h: 100 });
    expect(s.handle({ t: "action", action: "saveAs" })).toEqual([{ t: "export", kind: "saveAs" }]);
    expect(s.handle({ t: "restore", display: 9, rect: { x: 0, y: 0, w: 1, h: 1 } })).toEqual([]);
  });
});
