// The single render() (D-03): crops one display's frozen frame to the selection
// and draws the annotation document on top, onto ANY Canvas2D. The overlay calls
// it for the live preview and the export path calls it onto an OffscreenCanvas,
// then hands raw RGBA to Rust — so what you see is byte-for-byte what you copy.
//
// Shape coordinates are selection-local physical px (ARCHITECTURE Pattern 5).
// Blur samples the ORIGINAL crop bytes, never the already-annotated canvas, so it
// is non-destructive and undoable. Every shape is wrapped in save()/restore().
import type { AnnotationDocument, Shape } from "../annotate/document";
import type { PhysicalRect } from "../geometry";
import type { Canvas2D, Canvas2DDrawing, FrameRgba, ImageDataLike } from "./canvas";

/** Alpha of the Marker tool (ARCHITECTURE Pattern 5: Marker is drawn at alpha 0.5). */
const MARKER_ALPHA = 0.5;
/** Marker stroke is wider than the pen for the same `size`. */
const MARKER_WIDTH_FACTOR = 3;
/** Text font size in px per unit of `size` (size 3 → 12 px by default). */
export const TEXT_PX_PER_SIZE = 4;
/** Line height multiplier for multi-line text. */
export const TEXT_LINE_HEIGHT = 1.25;

/** Font pixel size of a text shape of `size` — shared with the overlay's text box so WYSIWYG holds. */
export function textFontPx(size: number): number {
  return size * TEXT_PX_PER_SIZE;
}

/** The CSS font of a text shape (same family in the box and on the canvas). */
export function textFont(size: number): string {
  return `${textFontPx(size)}px Arial, Helvetica, sans-serif`;
}

/**
 * `pending` is the in-progress stroke (CaptureSession.pendingShape()) drawn after the
 * committed shapes for the live preview; export passes nothing.
 */
export function render<I extends ImageDataLike>(
  frame: FrameRgba,
  selection: PhysicalRect,
  doc: AnnotationDocument,
  ctx: Canvas2D<I>,
  pending: Shape | null = null,
): void {
  const sel = clampToFrame(selection, frame);
  if (sel.w < 1 || sel.h < 1) return;

  // 1. Crop: copy the selection rows out of the frame into an ImageData at (0,0).
  const crop = ctx.createImageData(sel.w, sel.h);
  const rowBytes = sel.w * 4;
  for (let row = 0; row < sel.h; row += 1) {
    const src = ((sel.y + row) * frame.width + sel.x) * 4;
    crop.data.set(frame.data.subarray(src, src + rowBytes), row * rowBytes);
  }
  // Keep the untouched bytes for blur sampling before anything draws over them.
  const original = Uint8ClampedArray.from(crop.data);
  ctx.putImageData(crop, 0, 0);

  // 2. Shapes, in commit order.
  for (const s of doc.shapes) {
    ctx.save();
    drawShape(s, ctx, original, sel.w, sel.h);
    ctx.restore();
  }
  if (pending) {
    ctx.save();
    drawShape(pending, ctx, original, sel.w, sel.h);
    ctx.restore();
  }
}

function drawShape(s: Shape, ctx: Canvas2DDrawing, original: Uint8ClampedArray, w: number, h: number): void {
  switch (s.kind) {
    case "pen":
      stroke(ctx, s.pts, s.color, s.size, "round", 1);
      return;
    case "line":
      stroke(ctx, [s.a, s.b], s.color, s.size, "round", 1);
      return;
    case "arrow": {
      stroke(ctx, [s.a, s.b], s.color, s.size, "round", 1);
      // Filled triangle head at `b`, max(8, size*4) long, pointing along a→b.
      const len = Math.max(8, s.size * 4);
      const dx = s.b.x - s.a.x;
      const dy = s.b.y - s.a.y;
      const d = Math.hypot(dx, dy) || 1;
      const ux = dx / d;
      const uy = dy / d;
      const bx = s.b.x - ux * len;
      const by = s.b.y - uy * len;
      const half = len / 2;
      ctx.fillStyle = s.color;
      ctx.beginPath();
      ctx.moveTo(s.b.x, s.b.y);
      ctx.lineTo(bx - uy * half, by + ux * half);
      ctx.lineTo(bx + uy * half, by - ux * half);
      ctx.closePath();
      ctx.fill();
      return;
    }
    case "rect":
      ctx.strokeStyle = s.color;
      ctx.lineWidth = s.size;
      ctx.lineJoin = "miter";
      ctx.strokeRect(s.r.x, s.r.y, s.r.w, s.r.h);
      return;
    case "ellipse":
      ctx.strokeStyle = s.color;
      ctx.lineWidth = s.size;
      ctx.beginPath();
      ctx.ellipse(s.r.x + s.r.w / 2, s.r.y + s.r.h / 2, s.r.w / 2, s.r.h / 2, 0, 0, Math.PI * 2);
      ctx.stroke();
      return;
    case "marker":
      stroke(ctx, s.pts, s.color, s.size * MARKER_WIDTH_FACTOR, "square", MARKER_ALPHA);
      return;
    case "blur":
      pixelate(ctx, s.r, s.block, original, w, h);
      return;
    case "text": {
      // One font size per text box (`size` × TEXT_PX_PER_SIZE); `at` is the box's top-left,
      // lines are split on "\n" (the text box commits multi-line input).
      ctx.fillStyle = s.color;
      const px = textFontPx(s.size);
      ctx.font = textFont(s.size);
      ctx.textBaseline = "top";
      const lines = s.text.split("\n");
      for (let i = 0; i < lines.length; i += 1) {
        ctx.fillText(lines[i]!, s.at.x, s.at.y + i * Math.round(px * TEXT_LINE_HEIGHT));
      }
      return;
    }
    default: {
      const _exhaustive: never = s;
      return _exhaustive;
    }
  }
}

function stroke(
  ctx: Canvas2DDrawing,
  pts: readonly { x: number; y: number }[],
  color: string,
  width: number,
  cap: "round" | "square",
  alpha: number,
): void {
  const first = pts[0];
  if (!first) return;
  ctx.strokeStyle = color;
  ctx.lineWidth = width;
  ctx.lineCap = cap;
  ctx.lineJoin = "round";
  ctx.globalAlpha = alpha;
  ctx.beginPath();
  ctx.moveTo(first.x, first.y);
  if (pts.length === 1) ctx.lineTo(first.x, first.y); // a single click still leaves a dot
  for (let i = 1; i < pts.length; i += 1) ctx.lineTo(pts[i]!.x, pts[i]!.y);
  ctx.stroke();
}

/**
 * Pixelate: for every `block`×`block` cell inside `r` (clipped to the crop),
 * average the ORIGINAL crop bytes and fill the cell with that colour.
 */
function pixelate(ctx: Canvas2DDrawing, r: PhysicalRect, block: number, original: Uint8ClampedArray, w: number, h: number): void {
  const b = Math.max(1, Math.floor(block));
  const x0 = Math.max(0, Math.floor(r.x));
  const y0 = Math.max(0, Math.floor(r.y));
  const x1 = Math.min(w, Math.ceil(r.x + r.w));
  const y1 = Math.min(h, Math.ceil(r.y + r.h));
  for (let cy = y0; cy < y1; cy += b) {
    const ch = Math.min(b, y1 - cy);
    for (let cx = x0; cx < x1; cx += b) {
      const cw = Math.min(b, x1 - cx);
      let rs = 0;
      let gs = 0;
      let bs = 0;
      for (let y = cy; y < cy + ch; y += 1) {
        let i = (y * w + cx) * 4;
        for (let x = 0; x < cw; x += 1, i += 4) {
          rs += original[i]!;
          gs += original[i + 1]!;
          bs += original[i + 2]!;
        }
      }
      const n = cw * ch;
      ctx.fillStyle = `rgb(${Math.round(rs / n)},${Math.round(gs / n)},${Math.round(bs / n)})`;
      ctx.fillRect(cx, cy, cw, ch);
    }
  }
}

/** Intersect the selection with the frame so a stale or overhanging rect never reads out of bounds. */
function clampToFrame(sel: PhysicalRect, frame: FrameRgba): PhysicalRect {
  const x = Math.min(Math.max(0, sel.x), frame.width);
  const y = Math.min(Math.max(0, sel.y), frame.height);
  const w = Math.min(sel.x + sel.w, frame.width) - x;
  const h = Math.min(sel.y + sel.h, frame.height) - y;
  return { x, y, w: Math.max(0, w), h: Math.max(0, h) };
}
