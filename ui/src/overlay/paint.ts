// Dim layer painting (ARCHITECTURE Pattern 3): the full-window `#dim` canvas holds the
// 50 % veil with an undimmed hole for the selection, a crisp 1-px border and the 8 resize
// handles. The frozen frame itself is never painted here (native layer beneath the page on
// macOS; `#frame` canvas on Windows). All coordinates are this display's PHYSICAL pixels.
import type { PhysicalRect } from "../core/geometry";
import { DIM_FILL, HANDLE_FILL, HANDLE_HALF_PT, HANDLE_STROKE, SELECTION_BORDER } from "../theme/overlay";

export interface DimGeometry {
  width: number;
  height: number;
  scale: number;
}

/** Size `dim` to the display bitmap and fill the plain veil (no hole). */
export function paintDim(dim: HTMLCanvasElement, g: DimGeometry): void {
  dim.width = g.width;
  dim.height = g.height;
  dim.style.width = `${g.width / g.scale}px`;
  dim.style.height = `${g.height / g.scale}px`;
  paintDimWithHole(dim, null, g.scale, false);
}

/**
 * Repaint the veil with a hole for `rect` (border + handles when `handles`). A null rect
 * is the plain veil. The bitmap size is left as `paintDim` set it.
 */
export function paintDimWithHole(dim: HTMLCanvasElement, rect: PhysicalRect | null, scale: number, handles: boolean): void {
  const ctx = dim.getContext("2d");
  if (!ctx) throw new Error("2D context unavailable on the dim canvas");
  const { width, height } = dim;
  ctx.clearRect(0, 0, width, height);
  ctx.fillStyle = DIM_FILL;
  ctx.fillRect(0, 0, width, height);
  if (!rect || rect.w <= 0 || rect.h <= 0) return;
  ctx.clearRect(rect.x, rect.y, rect.w, rect.h);
  ctx.strokeStyle = SELECTION_BORDER;
  ctx.lineWidth = 1;
  ctx.strokeRect(rect.x + 0.5, rect.y + 0.5, Math.max(0, rect.w - 1), Math.max(0, rect.h - 1));
  if (!handles) return;
  const half = Math.max(2, Math.round(HANDLE_HALF_PT * scale));
  const L = rect.x;
  const T = rect.y;
  const R = rect.x + rect.w;
  const B = rect.y + rect.h;
  const cx = Math.round(L + rect.w / 2);
  const cy = Math.round(T + rect.h / 2);
  ctx.fillStyle = HANDLE_FILL;
  ctx.strokeStyle = HANDLE_STROKE;
  for (const [x, y] of [
    [L, T],
    [cx, T],
    [R, T],
    [R, cy],
    [R, B],
    [cx, B],
    [L, B],
    [L, cy],
  ] as const) {
    ctx.fillRect(x - half, y - half, half * 2, half * 2);
    ctx.strokeRect(x - half + 0.5, y - half + 0.5, half * 2 - 1, half * 2 - 1);
  }
}
