// CaptureSession — the pure, event-sourced state machine behind the overlay
// (D-02, D-06; ARCHITECTURE Pattern 4 "Session as a pure event-sourced state machine").
//
// The overlay adapter converts OS events into `OverlayInput` (display-local physical
// px) and repaints on `sceneChanged`; the controller performs exports on `export`
// and tears the overlay down on `dismiss`. `handle()` is a pure transition: no
// timers, no DOM, no globals, no OS or Tauri import — the same class runs under a
// WKWebView, WebView2, or a Vitest script against the fake platform.
//
// Keyboard rules live in `KEYMAP` below, one row per rule with its source, because
// Phase 2/3 extend it and Phase 5 must keep it identical on Windows: `ctrl` is
// treated exactly like `meta` ("primary"), so ⌘C on macOS and Ctrl+C on Windows
// are the same row (spec §5 "Hotkey (Win / Mac)").
import { AnnotationDocument, type Shape } from "./annotate/document";
import { ToolState, type ToolId } from "./annotate/tools";
import type { DisplayId, DisplayInfo, DisplayTopology, PhysicalPoint, PhysicalRect } from "./geometry";
import type { FrameRgba } from "./render/canvas";
import { SelectionModel, type Mods } from "./selection/model";

export type OverlayInput =
  | { t: "pointerDown"; display: DisplayId; p: PhysicalPoint; mods: Mods }
  | { t: "pointerMove"; display: DisplayId; p: PhysicalPoint }
  | { t: "pointerUp"; display: DisplayId; p: PhysicalPoint }
  /**
   * `display` is the panel the key arrived on; it only matters when no selection
   * exists yet (⌘A then selects that whole display — Phase 2 SELECT-05).
   */
  | { t: "key"; key: string; mods: Mods; display?: DisplayId }
  | { t: "wheel"; deltaY: number }
  | { t: "tool"; tool: ToolId | null }
  | { t: "action"; action: SessionAction; display?: DisplayId }
  /** The IME-hosted text box committed `text` at selection-local `at` (Phase 3, ANNOT-08). */
  | { t: "commitText"; at: PhysicalPoint; text: string }
  /** SELECT-08 "Remember the selected area": pre-select `rect` on `display`. */
  | { t: "restore"; display: DisplayId; rect: PhysicalRect };

/** Toolbar / context-menu actions (spec §5 plus "Select full screen" and "Clear selection"). */
export type SessionAction = "copy" | "save" | "saveAs" | "print" | "close" | "undo" | "fullscreen" | "clear";

export type SessionEvent =
  | { t: "sceneChanged"; display: DisplayId }
  | { t: "export"; kind: "clipboard" | "save" | "saveAs" | "print" }
  /** The Text tool was clicked inside the selection: host a text box at selection-local `at`. */
  | { t: "beginText"; display: DisplayId; at: PhysicalPoint }
  | { t: "dismiss" };

/** One keyboard rule: `key` is the DOM `KeyboardEvent.key` (letters lower-cased). */
export interface KeymapRow {
  key: string;
  mods: "none" | "primary" | "shift";
  action: string;
  source: string;
}

/**
 * The keyboard rules this session implements. "primary" = ⌘ on macOS, Ctrl on
 * Windows. Source: docs/SPEC.md (rows marked `DEFAULT` are choices the spec leaves
 * open).
 */
export const KEYMAP: readonly KeymapRow[] = [
  { key: "Escape", mods: "none", action: "dismiss (abort, desktop unchanged)", source: "spec §3.7 (DEFAULT: close the overlay)" },
  { key: "x", mods: "primary", action: "dismiss", source: "spec §3.7 'Ctrl+X / ⌘X aborts'" },
  { key: "a", mods: "primary", action: "toggle full-display selection, restoring the previous rect", source: "spec §3.8" },
  { key: "ArrowLeft|ArrowRight|ArrowUp|ArrowDown", mods: "none", action: "nudge the selection 1 px", source: "spec §3.4 (DEFAULT 1 px; no fast step)" },
  { key: "ArrowLeft|ArrowRight|ArrowUp|ArrowDown", mods: "shift", action: "resize 1 px on the right/bottom edge", source: "spec §3.4 'Shift+arrows resize by 1px'; edge unnamed — DEFAULT right/bottom" },
  { key: "z", mods: "primary", action: "undo the last annotation", source: "spec §4 Undo 'Ctrl+Z / ⌘Z', no redo" },
  { key: "c", mods: "primary", action: "export to clipboard", source: "spec §5 Copy 'Ctrl+C / ⌘C'" },
  { key: "s", mods: "primary", action: "export save (instant save to the last folder is the Phase 3 exporter's job; Windows rule, D-25)", source: "spec §5 Save 'Ctrl+S / ⌘S'" },
  { key: "p", mods: "primary", action: "export print", source: "spec §5 Print 'Ctrl+P / ⌘P'" },
  { key: "wheel", mods: "none", action: "tool size ±1 while a tool is active (deltaY > 0 = wheel down = bigger)", source: "spec §4 'wheel down = bigger, wheel up = smaller, with min/max limits'" },
  { key: "=|+|-", mods: "primary", action: "tool size ±1 while a tool is active (keyboard twin of the wheel)", source: "REQUIREMENTS ANNOT-10 'Mouse wheel (and Ctrl + / Ctrl -) changes stroke, arrow, and text size'" },
];

/** Block size of the Blur (pixelate) tool in physical px — DEFAULT, Windows-only tool (D-25). */
const BLUR_BLOCK = 8;

/** A stroke in progress between pointerDown and pointerUp, in selection-local px. */
interface Stroke {
  tool: ToolId;
  pts: PhysicalPoint[];
}

export class CaptureSession {
  /** Immutable for the life of the session (ARCHITECTURE §State Management). */
  readonly topology: DisplayTopology;
  /** One frozen frame per display, keyed by DisplayId. */
  readonly frames: Map<DisplayId, FrameRgba>;
  /** Shared tool/colour/size state (persists across sessions via Settings later). */
  readonly tool: ToolState;
  /** Annotations of the current selection, selection-local px. */
  readonly doc = new AnnotationDocument();
  private home: DisplayId | null = null;
  private model: SelectionModel | null = null;
  private stroke: Stroke | null = null;

  constructor(topology: DisplayTopology, frames: Map<DisplayId, FrameRgba>, tool?: ToolState) {
    this.topology = topology;
    this.frames = frames;
    this.tool = tool ?? new ToolState();
  }

  /** The display that owns the selection (ARCHITECTURE Pattern 2), or null before the first click. */
  get homeDisplay(): DisplayId | null {
    return this.home;
  }

  selection(): { display: DisplayId; model: SelectionModel } | null {
    return this.home !== null && this.model ? { display: this.home, model: this.model } : null;
  }

  /** Pure transition: consume one input, return the events it produced. */
  handle(input: OverlayInput): SessionEvent[] {
    switch (input.t) {
      case "pointerDown":
        return this.pointerDown(input.display, input.p, input.mods);
      case "pointerMove":
        return this.pointerMove(input.display, input.p);
      case "pointerUp":
        return this.pointerUp(input.display, input.p);
      case "key":
        return this.key(input.key, input.mods, input.display);
      case "wheel":
        return this.wheel(input.deltaY);
      case "tool":
        this.tool.active = input.tool;
        this.stroke = null;
        return this.home === null ? [] : [this.scene()];
      case "action":
        return this.action(input.action, input.display);
      case "commitText":
        return this.commitText(input.at, input.text);
      case "restore":
        return this.restore(input.display, input.rect);
      default: {
        const _exhaustive: never = input;
        return _exhaustive;
      }
    }
  }

  // --- pointer -------------------------------------------------------------

  /**
   * Tool-first hit testing (ARCHITECTURE Pattern 4; ANNOT-12): with a tool armed, a
   * press INSIDE the selection starts a shape; a press on a handle still resizes;
   * anything else goes to the selection model (move / new drag). A press on a
   * display other than the home display re-homes the session — the selection and
   * its annotations are replaced (spec §3.4: a selection lives on exactly one display).
   */
  private pointerDown(display: DisplayId, p: PhysicalPoint, mods: Mods): SessionEvent[] {
    const info = this.display(display);
    if (!info) return [];
    if (this.home !== display || !this.model) {
      this.home = display;
      this.model = new SelectionModel({ w: info.physical.w, h: info.physical.h });
      this.doc.clear();
    }
    const model = this.model;
    const rect = model.rect();
    const handle = model.hitTest(p);
    if (this.tool.active && rect && handle === "body" && model.state.kind === "selected") {
      if (this.tool.active === "text") {
        // Text is placed by an IME-hosted text box the overlay owns (ANNOT-08), not by a drag;
        // the overlay commits it back through `commitText`.
        return [{ t: "beginText", display, at: local(p, rect) }];
      }
      this.stroke = { tool: this.tool.active, pts: [local(p, rect)] };
      return [this.scene()];
    }
    if (handle === "outside") this.doc.clear(); // a new selection discards selection-local shapes
    model.pointerDown(p, mods);
    return [this.scene()];
  }

  private pointerMove(display: DisplayId, p: PhysicalPoint): SessionEvent[] {
    if (this.home !== display || !this.model) return [];
    if (this.stroke) {
      const rect = this.model.rect();
      if (rect) pushPoint(this.stroke.pts, local(p, rect));
      return [this.scene()];
    }
    const kind = this.model.state.kind;
    if (kind !== "dragging" && kind !== "moving" && kind !== "resizing") return [];
    this.model.pointerMove(p);
    return [this.scene()];
  }

  /**
   * Release: finish a stroke into the document, or commit the drag. ⌘/Ctrl-drag
   * quick copy (spec §3.6): on release of a new drag started with the primary
   * modifier, export to the clipboard and close with no toolbar.
   */
  private pointerUp(display: DisplayId, p: PhysicalPoint): SessionEvent[] {
    if (this.home !== display || !this.model) return [];
    if (this.stroke) {
      const rect = this.model.rect();
      if (rect) {
        pushPoint(this.stroke.pts, local(p, rect));
        this.doc.commit(shapeFrom(this.stroke, this.tool));
      }
      this.stroke = null;
      return [this.scene()];
    }
    const kind = this.model.state.kind;
    if (kind !== "dragging" && kind !== "moving" && kind !== "resizing") return [];
    const { quickCopy } = this.model.pointerUp(p);
    // A zero-area release leaves no rect (idle), so there is nothing to copy.
    if (quickCopy && this.model.rect()) {
      return [{ t: "export", kind: "clipboard" }, { t: "dismiss" }];
    }
    return [this.scene()];
  }

  // --- keyboard / wheel / actions -------------------------------------------

  /**
   * Keyboard rules (KEYMAP). A selection-dependent key pressed before any selection
   * exists is ignored (the overlay must never throw on a stray ⌘C), except ⌘A, which
   * selects the whole display the key arrived on (SELECT-05).
   */
  private key(rawKey: string, mods: Mods, display?: DisplayId): SessionEvent[] {
    const primary = mods.meta || mods.ctrl;
    const key = rawKey.length === 1 ? rawKey.toLowerCase() : rawKey;
    if (key === "Escape") return [{ t: "dismiss" }];
    if (primary && key === "x") return [{ t: "dismiss" }];
    if (primary && key === "z") return this.undo();
    if (primary && key === "c") return this.exportKind("clipboard");
    if (primary && key === "s") return this.exportKind("save");
    if (primary && key === "p") return this.exportKind("print");
    if (primary && key === "a") return this.fullscreen(display);
    if (primary && (key === "=" || key === "+")) return this.sizeBy(1);
    if (primary && key === "-") return this.sizeBy(-1);
    const arrow = ARROWS[key];
    if (arrow) {
      const model = this.model;
      if (!model || this.home === null) return [];
      if (mods.shift) model.resizeBy(arrow[0], arrow[1]);
      else model.nudge(arrow[0], arrow[1]);
      return [this.scene()];
    }
    return [];
  }

  /** ⌘A / double-click / context-menu "Fullscreen": toggle the full home display, or select the whole `display` when nothing is selected yet. */
  private fullscreen(display?: DisplayId): SessionEvent[] {
    if (!this.model || this.home === null) {
      if (display === undefined) return [];
      const info = this.display(display);
      if (!info) return [];
      this.home = display;
      this.model = new SelectionModel({ w: info.physical.w, h: info.physical.h });
      this.doc.clear();
    }
    this.model.toggleFullscreen();
    return [this.scene()];
  }

  /** Ctrl/⌘ + / − : the keyboard twin of the wheel (ANNOT-10); only while a tool is active. */
  private sizeBy(delta: number): SessionEvent[] {
    if (!this.tool.active) return [];
    const before = this.tool.size;
    this.tool.adjustSize(delta);
    if (this.tool.size === before || this.home === null) return [];
    return [this.scene()];
  }

  /** Spec §4: the wheel changes the size only while a tool is active; deltaY > 0 (wheel down) = bigger. */
  private wheel(deltaY: number): SessionEvent[] {
    if (!this.tool.active || deltaY === 0) return [];
    const before = this.tool.size;
    this.tool.adjustSize(deltaY > 0 ? 1 : -1);
    if (this.tool.size === before || this.home === null) return [];
    return [this.scene()];
  }

  private action(action: SessionAction, display?: DisplayId): SessionEvent[] {
    switch (action) {
      case "copy":
        return this.exportKind("clipboard");
      case "save":
        return this.exportKind("save");
      case "saveAs":
        return this.exportKind("saveAs");
      case "print":
        return this.exportKind("print");
      case "close":
        return [{ t: "dismiss" }];
      case "undo":
        return this.undo();
      case "fullscreen":
        return this.fullscreen(display);
      case "clear": {
        // "Clear selection" (spec §3.9): back to the no-selection state on the home display.
        if (!this.model || this.home === null) return [];
        this.model.clear();
        this.doc.clear();
        this.stroke = null;
        return [this.scene()];
      }
    }
  }

  private undo(): SessionEvent[] {
    if (!this.model || this.home === null) return [];
    return this.doc.undo() ? [this.scene()] : [];
  }

  /** Export only when a committed rect exists; otherwise the key/button is a no-op. */
  private exportKind(kind: "clipboard" | "save" | "saveAs" | "print"): SessionEvent[] {
    if (!this.model || this.home === null || !this.model.rect() || this.model.state.kind !== "selected") return [];
    return [{ t: "export", kind }];
  }

  /** ANNOT-08: the text box committed. Empty or whitespace-only text leaves no shape. */
  private commitText(at: PhysicalPoint, text: string): SessionEvent[] {
    if (!this.model || this.home === null) return [];
    if (text.trim().length === 0) return [this.scene()];
    this.doc.commit({ kind: "text", at, text, color: this.tool.colorFor("text"), size: this.tool.size });
    return [this.scene()];
  }

  /** SELECT-08: pre-select `rect` (clamped) on `display` before the first pointer event. */
  private restore(display: DisplayId, rect: PhysicalRect): SessionEvent[] {
    const info = this.display(display);
    if (!info) return [];
    const model = new SelectionModel({ w: info.physical.w, h: info.physical.h });
    const w = Math.min(Math.max(1, rect.w), info.physical.w);
    const h = Math.min(Math.max(1, rect.h), info.physical.h);
    const x = Math.min(Math.max(0, rect.x), info.physical.w - w);
    const y = Math.min(Math.max(0, rect.y), info.physical.h - h);
    model.state = { kind: "selected", rect: { x, y, w, h } };
    this.home = display;
    this.model = model;
    this.doc.clear();
    this.stroke = null;
    return [this.scene()];
  }

  /**
   * The shape being drawn between pointerDown and pointerUp (selection-local), for the
   * live preview; `null` when no stroke is in progress. Never committed by this call.
   */
  pendingShape(): Shape | null {
    if (!this.stroke || this.stroke.pts.length === 0) return null;
    return shapeFrom(this.stroke, this.tool);
  }

  // --- helpers --------------------------------------------------------------

  private scene(): SessionEvent {
    if (this.home === null) throw new Error("no home display");
    return { t: "sceneChanged", display: this.home };
  }

  private display(id: DisplayId): DisplayInfo | undefined {
    return this.topology.displays.find((d) => d.id === id);
  }
}

const ARROWS: Record<string, readonly [-1 | 0 | 1, -1 | 0 | 1]> = {
  ArrowLeft: [-1, 0],
  ArrowRight: [1, 0],
  ArrowUp: [0, -1],
  ArrowDown: [0, 1],
};

/** Append a stroke point, skipping a repeat of the last one (move + up at the same spot). */
function pushPoint(pts: PhysicalPoint[], p: PhysicalPoint): void {
  const last = pts[pts.length - 1];
  if (last && last.x === p.x && last.y === p.y) return;
  pts.push(p);
}

/** Display-local → selection-local px (ARCHITECTURE Pattern 5: shapes are selection-local). */
function local(p: PhysicalPoint, rect: PhysicalRect): PhysicalPoint {
  return { x: p.x - rect.x, y: p.y - rect.y };
}

function shapeFrom(stroke: Stroke, tool: ToolState): Shape {
  const pts = stroke.pts;
  const a = pts[0]!;
  const b = pts[pts.length - 1]!;
  const color = tool.colorFor(stroke.tool);
  const size = tool.size;
  switch (stroke.tool) {
    case "pen":
      return { kind: "pen", pts, color, size };
    case "marker":
      return { kind: "marker", pts, color, size };
    case "line":
      return { kind: "line", a, b, color, size };
    case "arrow":
      return { kind: "arrow", a, b, color, size };
    case "rect":
      return { kind: "rect", r: normalise(a, b), color, size };
    case "ellipse":
      return { kind: "ellipse", r: normalise(a, b), color, size };
    case "blur":
      return { kind: "blur", r: normalise(a, b), block: BLUR_BLOCK };
    case "text":
      return { kind: "text", at: a, text: "", color, size };
    default: {
      const _exhaustive: never = stroke.tool;
      return _exhaustive;
    }
  }
}

function normalise(a: PhysicalPoint, b: PhysicalPoint): PhysicalRect {
  return {
    x: Math.min(a.x, b.x),
    y: Math.min(a.y, b.y),
    w: Math.max(1, Math.abs(a.x - b.x)),
    h: Math.max(1, Math.abs(a.y - b.y)),
  };
}
