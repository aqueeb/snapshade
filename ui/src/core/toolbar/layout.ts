// Toolbar placement (spec §3.5, ANNOT-14). The side order: the tool strip hugs the selection's
// RIGHT edge, top-aligned; the action strip hugs the BOTTOM edge, right-aligned. When a
// strip does not fit on its side it flips to the opposite side, and when neither side
// fits it moves inside the selection (the "hugging the inside edge" variant).
//
// Pure geometry in the display's CSS-pixel space (points): the DOM positions strips in
// CSS px, so callers convert the physical selection once with `physicalToCss`.
import type { PhysicalRect } from "../geometry";

export interface Size {
  w: number;
  h: number;
}

/** A rectangle in CSS px (points) of one display's panel. */
export interface CssRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export type ToolsSide = "right" | "left" | "inside";
export type ActionsSide = "below" | "above" | "inside";

export interface ToolbarLayout {
  tools: { x: number; y: number; side: ToolsSide };
  actions: { x: number; y: number; side: ActionsSide };
}

export interface ToolbarLayoutInput {
  /** The selection in CSS px. */
  sel: CssRect;
  /** The panel (display) size in CSS px. */
  bounds: Size;
  /** Rendered size of the vertical tool strip. */
  tools: Size;
  /** Rendered size of the horizontal action strip. */
  actions: Size;
  /** Gap between the selection edge and a strip. */
  gap: number;
}

/** Physical selection → CSS px rect of the same display (`scale` = devicePixelRatio). */
export function physicalToCss(r: PhysicalRect, scale: number): CssRect {
  return { x: r.x / scale, y: r.y / scale, w: r.w / scale, h: r.h / scale };
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.min(Math.max(v, lo), Math.max(lo, hi));
}

export function layoutToolbars(input: ToolbarLayoutInput): ToolbarLayout {
  const { sel, bounds, tools, actions, gap } = input;
  const right = sel.x + sel.w;
  const bottom = sel.y + sel.h;

  // Vertical tool strip: right of the selection, else left, else inside the right edge.
  let toolsSide: ToolsSide;
  let tx: number;
  if (right + gap + tools.w <= bounds.w) {
    toolsSide = "right";
    tx = right + gap;
  } else if (sel.x - gap - tools.w >= 0) {
    toolsSide = "left";
    tx = sel.x - gap - tools.w;
  } else {
    toolsSide = "inside";
    tx = right - gap - tools.w;
  }
  // Top-aligned with the selection, kept on screen.
  const ty = clamp(sel.y, 0, bounds.h - tools.h);

  // Horizontal action strip: below the selection, else above, else inside the bottom edge.
  let actionsSide: ActionsSide;
  let ay: number;
  if (bottom + gap + actions.h <= bounds.h) {
    actionsSide = "below";
    ay = bottom + gap;
  } else if (sel.y - gap - actions.h >= 0) {
    actionsSide = "above";
    ay = sel.y - gap - actions.h;
  } else {
    actionsSide = "inside";
    ay = bottom - gap - actions.h;
  }
  // Right-aligned with the selection, kept on screen.
  const ax = clamp(right - actions.w, 0, bounds.w - actions.w);

  // When both strips are inside they may overlap in the bottom-right corner; lift the
  // action strip above the tool strip's bottom in that one case.
  if (toolsSide === "inside" && actionsSide === "inside") {
    const toolsBottom = ty + tools.h;
    if (ay < toolsBottom && ax + actions.w > tx) {
      ay = clamp(toolsBottom + gap, 0, bounds.h - actions.h);
    }
  }

  return {
    tools: { x: Math.round(tx), y: Math.round(ty), side: toolsSide },
    actions: { x: Math.round(ax), y: Math.round(ay), side: actionsSide },
  };
}

/**
 * Where the W×H size badge goes (spec §3.4): above the top-left corner, below it when the
 * selection touches the top, clamped horizontally and vertically
 * (01-UI-REVIEW fix 2/P1 vertical clamp)).
 */
export function layoutBadge(sel: CssRect, bounds: Size, badge: Size, gap: number): { x: number; y: number } {
  let y = sel.y - gap - badge.h;
  if (y < 0) y = sel.y + gap;
  y = clamp(y, 0, bounds.h - badge.h);
  const x = clamp(sel.x, 0, bounds.w - badge.w);
  return { x: Math.round(x), y: Math.round(y) };
}
