// Structural Canvas 2D contract (D-03, RESEARCH Q8): only the members `render()`
// uses, so both the browser `CanvasRenderingContext2D` (WKWebView overlay) and
// the napi-rs Skia canvas's `SKRSContext2D` (headless golden tests) are
// assignable without any import in core. `render.test.ts` proves the browser
// assignability at compile time.

/** The subset of `ImageData` the renderer reads and writes. */
export interface ImageDataLike {
  readonly width: number;
  readonly height: number;
  readonly data: Uint8ClampedArray;
}

/** Drawing-only members (no pixel transfer). Shape helpers take this narrower type. */
export interface Canvas2DDrawing {
  fillStyle: string | object;
  strokeStyle: string | object;
  lineWidth: number;
  lineCap: "butt" | "round" | "square";
  lineJoin: "round" | "bevel" | "miter";
  globalAlpha: number;
  font: string;
  textBaseline: "top" | "hanging" | "middle" | "alphabetic" | "ideographic" | "bottom";
  beginPath(): void;
  moveTo(x: number, y: number): void;
  lineTo(x: number, y: number): void;
  closePath(): void;
  stroke(): void;
  fill(): void;
  rect(x: number, y: number, w: number, h: number): void;
  ellipse(
    x: number,
    y: number,
    radiusX: number,
    radiusY: number,
    rotation: number,
    startAngle: number,
    endAngle: number,
  ): void;
  strokeRect(x: number, y: number, w: number, h: number): void;
  fillRect(x: number, y: number, w: number, h: number): void;
  fillText(text: string, x: number, y: number): void;
  save(): void;
  restore(): void;
  clip(): void;
}

/**
 * The full contract `render()` needs. `I` is the host's concrete `ImageData`
 * type (the DOM `ImageData` in the browser, napi-rs's in tests) so the
 * create → put round trip type-checks against either host.
 */
export interface Canvas2D<I extends ImageDataLike = ImageDataLike> extends Canvas2DDrawing {
  createImageData(w: number, h: number): I;
  putImageData(img: I, dx: number, dy: number): void;
  getImageData(x: number, y: number, w: number, h: number): I;
}

/** One display's frozen frame in physical pixels, RGBA8, row-major, no padding. */
export interface FrameRgba {
  width: number;
  height: number;
  data: Uint8ClampedArray;
}
