// SelectionModel — the selection state machine over ONE display's
// physical pixels (D-02; ARCHITECTURE Pattern 2 "Selection has a home display").
//
// Every rule below cites `docs/SPEC.md` (§3.x). Choices the spec leaves open are
// flagged `DEFAULT:`.
//
// Coordinates: a rect is `[x, x + w) × [y, y + h)` in display-local physical
// pixels; pointer positions are pixel-edge coordinates clamped to
// `[0, bounds.w] × [0, bounds.h]`, so a selection can reach the last column.
// The model has no OS, Tauri, or DOM imports (D-06, D-19).
import type { PhysicalPoint, PhysicalRect, PhysicalSize } from "../geometry";

/** Hit-test result: one of the 8 resize handles, the selection body, or nothing. */
export type Handle = "n" | "ne" | "e" | "se" | "s" | "sw" | "w" | "nw" | "body" | "outside";

/**
 * Selection states (ARCHITECTURE Pattern 4 `SelState`). `state` is replaced,
 * never mutated, on every transition so tests and the overlay can snapshot it.
 * `moving` / `resizing` carry the live `rect` so `rect()` and the renderer
 * never have to recompute it.
 */
export type SelState =
  | { kind: "idle" }
  | { kind: "dragging"; anchor: PhysicalPoint; cur: PhysicalPoint; quickCopy: boolean }
  | { kind: "selected"; rect: PhysicalRect }
  | { kind: "moving"; start: PhysicalRect; grab: PhysicalPoint; rect: PhysicalRect }
  | { kind: "resizing"; start: PhysicalRect; handle: Handle; grab: PhysicalPoint; rect: PhysicalRect };

/** Modifier keys as the overlay reports them; `ctrl` is treated like `meta` (Windows parity). */
export interface Mods {
  meta: boolean;
  ctrl: boolean;
  shift: boolean;
  alt: boolean;
}

const RESIZE_HANDLES: readonly Handle[] = ["nw", "ne", "se", "sw", "n", "e", "s", "w"];

export class SelectionModel {
  /** Physical size of the home display; every rect is clamped inside it. */
  readonly bounds: PhysicalSize;
  /** Half-size of each handle square in px (hit slop). */
  readonly handleSlopPx: number;
  /** Current state; replaced on every transition. */
  state: SelState = { kind: "idle" };
  /**
   * Rect saved by the ⌘A toggle so a second ⌘A restores it (spec §3.8).
   */
  private preFullscreen: PhysicalRect | null = null;

  constructor(bounds: PhysicalSize, opts?: { handleSlopPx?: number }) {
    this.bounds = bounds;
    this.handleSlopPx = opts?.handleSlopPx ?? 8;
  }

  /** The current rect, live during a drag/move/resize, or `null` when idle. */
  rect(): PhysicalRect | null {
    const s = this.state;
    switch (s.kind) {
      case "idle":
        return null;
      case "dragging":
        return this.normalise(s.anchor, s.cur);
      case "selected":
      case "moving":
      case "resizing":
        return s.rect;
    }
  }

  /**
   * Which part of the selection is under `p`.
   * 8 handle squares (4 corners + 4 edge midpoints, each `±handleSlopPx`)
   * win over the body; inside the rect is `body`; anything else `outside`.
   * The inside/outside split is the spec §3.4 move-vs-new-selection rule.
   */
  hitTest(p: PhysicalPoint): Handle {
    const r = this.rect();
    if (!r || this.state.kind === "dragging") return "outside";
    const slop = this.handleSlopPx;
    const near = (a: number, b: number): boolean => Math.abs(a - b) <= slop;
    const L = r.x;
    const T = r.y;
    const R = r.x + r.w;
    const B = r.y + r.h;
    const cx = L + r.w / 2;
    const cy = T + r.h / 2;
    const squares: Record<Exclude<Handle, "body" | "outside">, PhysicalPoint> = {
      nw: { x: L, y: T },
      ne: { x: R, y: T },
      se: { x: R, y: B },
      sw: { x: L, y: B },
      n: { x: cx, y: T },
      e: { x: R, y: cy },
      s: { x: cx, y: B },
      w: { x: L, y: cy },
    };
    for (const h of RESIZE_HANDLES) {
      const sq = squares[h as keyof typeof squares];
      if (near(p.x, sq.x) && near(p.y, sq.y)) return h;
    }
    if (p.x >= L && p.x < R && p.y >= T && p.y < B) return "body";
    return "outside";
  }

  /**
   * Mouse-down decides between resize, move, and a new selection (spec §3.4):
   * - handle → `resizing` (remembers the start rect and grab point);
   * - body   → `moving` (the rect at drag start is offset, never re-anchored);
   * - outside / idle → `dragging` from an anchor; spec §3.4 "clicking outside the selection starts a new
   *            selection" (DEFAULT: no deselect-first step).
   * `quickCopy` is latched from ⌘ (or Ctrl on Windows) at mouse-down and only for a
   * new drag (spec §3.6: hold Ctrl/⌘ while STARTING the drag → copy on release).
   */
  pointerDown(p: PhysicalPoint, mods: Mods): void {
    const q = this.clampPoint(p);
    const handle = this.hitTest(q);
    const r = this.rect();
    this.preFullscreen = null;
    if (r && handle === "body") {
      this.state = { kind: "moving", start: r, grab: q, rect: r };
      return;
    }
    if (r && handle !== "outside") {
      this.state = { kind: "resizing", start: r, handle, grab: q, rect: r };
      return;
    }
    this.state = { kind: "dragging", anchor: q, cur: q, quickCopy: mods.meta || mods.ctrl };
  }

  /**
   * One routine updates the live rect for all three drag modes. Every result is
   * clamped to `bounds`; the selection never leaves its home display
   * (ARCHITECTURE Pattern 2).
   */
  pointerMove(p: PhysicalPoint): void {
    const q = this.clampPoint(p);
    const s = this.state;
    switch (s.kind) {
      case "dragging":
        this.state = { ...s, cur: q };
        return;
      case "moving": {
        const dx = q.x - s.grab.x;
        const dy = q.y - s.grab.y;
        this.state = { ...s, rect: this.clampRect({ ...s.start, x: s.start.x + dx, y: s.start.y + dy }) };
        return;
      }
      case "resizing":
        this.state = { ...s, rect: this.resizeFrom(s.start, s.handle, q.x - s.grab.x, q.y - s.grab.y) };
        return;
      default:
        return;
    }
  }

  /**
   * Mouse-up commits the drag: `dragging` → `selected` (spec §3.4), `moving` /
   * `resizing` → `selected` with the live rect. Returns the quick-copy flag so
   * the session can emit Export(clipboard) + Dismiss (spec §3.6).
   * DEFAULT: a zero-area drag (a plain click) leaves no selection.
   */
  pointerUp(p: PhysicalPoint): { quickCopy: boolean } {
    const q = this.clampPoint(p);
    const s = this.state;
    switch (s.kind) {
      case "dragging": {
        const r = this.normalise(s.anchor, q);
        this.state = r.w >= 1 && r.h >= 1 ? { kind: "selected", rect: r } : { kind: "idle" };
        return { quickCopy: s.quickCopy };
      }
      case "moving":
        this.state = { kind: "selected", rect: s.rect };
        return { quickCopy: false };
      case "resizing":
        this.state = { kind: "selected", rect: s.rect };
        return { quickCopy: false };
      default:
        return { quickCopy: false };
    }
  }

  /**
   * Arrow keys move the selection by exactly 1 px, clamped to the display
   * (spec §3.4). No fast step is exposed here; ⇧+arrows resize instead.
   */
  nudge(dx: -1 | 0 | 1, dy: -1 | 0 | 1): void {
    if (this.state.kind !== "selected") return;
    const r = this.state.rect;
    this.preFullscreen = null;
    this.state = { kind: "selected", rect: this.clampRect({ ...r, x: r.x + dx, y: r.y + dy }) };
  }

  /**
   * Shift+arrows resize by 1 px on the right/bottom edge, min 1×1, clamped
   * (spec §3.4 "Shift+arrows resize by 1px"; the edge is not named by the spec —
   * DEFAULT: right/bottom).
   */
  resizeBy(dw: -1 | 0 | 1, dh: -1 | 0 | 1): void {
    if (this.state.kind !== "selected") return;
    const r = this.state.rect;
    this.preFullscreen = null;
    const w = Math.min(Math.max(1, r.w + dw), this.bounds.w - r.x);
    const h = Math.min(Math.max(1, r.h + dh), this.bounds.h - r.y);
    this.state = { kind: "selected", rect: { x: r.x, y: r.y, w, h } };
  }

  /**
   * ⌘A / "Select full screen" is a toggle (spec §3.8): the current rect is saved and
   * a second toggle restores it. From idle it selects the whole display (nothing to
   * restore; a second toggle is a no-op — DEFAULT). Any other mutation in between drops the
   * saved rect, so ⌘A after a nudge selects the full display again.
   */
  toggleFullscreen(): void {
    const full: PhysicalRect = { x: 0, y: 0, w: this.bounds.w, h: this.bounds.h };
    const s = this.state;
    if (s.kind === "selected" && this.preFullscreen && sameRect(s.rect, full)) {
      this.state = { kind: "selected", rect: this.preFullscreen };
      this.preFullscreen = null;
      return;
    }
    this.preFullscreen = s.kind === "selected" && !sameRect(s.rect, full) ? s.rect : null;
    this.state = { kind: "selected", rect: full };
  }

  /** "Clear selection" (spec §3.9; `ScreenshotView -hideSelection`): back to idle. */
  clear(): void {
    this.state = { kind: "idle" };
    this.preFullscreen = null;
  }

  // --- helpers -------------------------------------------------------------

  private clampPoint(p: PhysicalPoint): PhysicalPoint {
    return {
      x: Math.min(Math.max(0, p.x), this.bounds.w),
      y: Math.min(Math.max(0, p.y), this.bounds.h),
    };
  }

  /** Keep a rect of fixed size inside the display. */
  private clampRect(r: PhysicalRect): PhysicalRect {
    const w = Math.min(r.w, this.bounds.w);
    const h = Math.min(r.h, this.bounds.h);
    return {
      x: Math.min(Math.max(0, r.x), this.bounds.w - w),
      y: Math.min(Math.max(0, r.y), this.bounds.h - h),
      w,
      h,
    };
  }

  /** Anchor + current pointer → normalised rect (`[min, max)` on each axis). */
  private normalise(a: PhysicalPoint, b: PhysicalPoint): PhysicalRect {
    const x = Math.min(a.x, b.x);
    const y = Math.min(a.y, b.y);
    return { x, y, w: Math.abs(a.x - b.x), h: Math.abs(a.y - b.y) };
  }

  /**
   * Move the edges a handle owns by (dx, dy); each edge is clamped so the rect
   * keeps at least 1×1 and stays inside the display (never flips).
   */
  private resizeFrom(start: PhysicalRect, handle: Handle, dx: number, dy: number): PhysicalRect {
    let L = start.x;
    let T = start.y;
    let R = start.x + start.w;
    let B = start.y + start.h;
    if (handle === "nw" || handle === "w" || handle === "sw") L = Math.min(Math.max(0, L + dx), R - 1);
    if (handle === "ne" || handle === "e" || handle === "se") R = Math.max(Math.min(this.bounds.w, R + dx), L + 1);
    if (handle === "nw" || handle === "n" || handle === "ne") T = Math.min(Math.max(0, T + dy), B - 1);
    if (handle === "sw" || handle === "s" || handle === "se") B = Math.max(Math.min(this.bounds.h, B + dy), T + 1);
    return { x: L, y: T, w: R - L, h: B - T };
  }
}

function sameRect(a: PhysicalRect, b: PhysicalRect): boolean {
  return a.x === b.x && a.y === b.y && a.w === b.w && a.h === b.h;
}
