// Deterministic synthetic frame for the renderer goldens (Plan 01-05 Task 2).
// A red/green gradient over blue 128 with a black 2 px grid every 8 px, so blur
// (uniform blocks) and marker (translucency over black lines) are visible.
// No randomness: the same (w, h) always yields the same bytes.
import type { FrameRgba } from "../src/core/render/canvas";

export function makeFrame(w: number, h: number): FrameRgba {
  const data = new Uint8ClampedArray(w * h * 4);
  for (let y = 0; y < h; y += 1) {
    for (let x = 0; x < w; x += 1) {
      const i = (y * w + x) * 4;
      const onGrid = x % 8 < 2 || y % 8 < 2;
      if (onGrid) {
        data[i] = 0;
        data[i + 1] = 0;
        data[i + 2] = 0;
      } else {
        data[i] = Math.round((x * 255) / Math.max(1, w - 1));
        data[i + 1] = Math.round((y * 255) / Math.max(1, h - 1));
        data[i + 2] = 128;
      }
      data[i + 3] = 255;
    }
  }
  return { width: w, height: h, data };
}
