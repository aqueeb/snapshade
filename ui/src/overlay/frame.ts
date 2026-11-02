// Frozen-frame loader: `frame://localhost/<display_id>?gen=<n>&token=<hex>` → RGBA bytes
// (`http://frame.localhost/…` on WebView2, see `frameOrigin`)
// (RESEARCH Q5; ARCHITECTURE Pattern 2; D-02/D-03 export path).
//
// Pixels never cross IPC as JSON/base64: this is a plain `fetch` of the frame body whose
// dimensions travel in response headers. No Tauri API import here — only fetch, so the
// loader is host-agnostic.
//
// Access (Plan 01-10; T-01-28): Rust serves pixels only to an allow-listed page origin AND
// only with the generation's capability token, which the page receives inside the
// `frame-ready` event and passes here verbatim. Anything else is a 404.
//
// Nothing on the hotkey path calls this: the frozen frame is composited natively under the
// transparent page (macOS). The page fetches the frame AFTER reporting painted, so the
// annotation preview (`render()` crops from these bytes) and the export have pixels.
import type { FrameRgba } from "../core/render/canvas";

/** Dimensions of the loaded frame: bitmap size in physical px, `scale` px per point. */
export interface FrameInfo {
  width: number;
  height: number;
  scale: number;
}

function headerNumber(res: Response, name: string): number {
  const raw = res.headers.get(name);
  const value = raw === null ? Number.NaN : Number(raw);
  if (!Number.isFinite(value) || value <= 0) {
    throw new Error(`frame:// response header ${name} is missing or invalid (${raw ?? "null"})`);
  }
  return value;
}

/**
 * The scheme's origin on this host: WKWebView serves a custom scheme as
 * `frame://localhost`; WebView2 (Windows) and Android serve it as `http://frame.localhost`
 * (Tauri's `use_https_scheme` is off). Decided from the user agent so this module keeps
 * its no-Tauri-import rule; the Rust side accepts both forms.
 */
export function frameOrigin(userAgent: string = typeof navigator === "undefined" ? "" : navigator.userAgent): string {
  return userAgent.includes("Windows") ? "http://frame.localhost" : "frame://localhost";
}

/** Fetch the raw RGBA frame for `displayId` at generation `gen` with the generation's `token`. */
export async function fetchFrame(displayId: number, gen: number, token: string): Promise<FrameRgba & FrameInfo> {
  const res = await fetch(`${frameOrigin()}/${displayId}?gen=${gen}&token=${encodeURIComponent(token)}`);
  if (!res.ok) {
    throw new Error(`frame:// fetch for display ${displayId} gen ${gen} failed: ${res.status}`);
  }
  const width = headerNumber(res, "X-Frame-Width");
  const height = headerNumber(res, "X-Frame-Height");
  const scale = headerNumber(res, "X-Frame-Scale");
  const buf = await res.arrayBuffer();
  const expected = width * height * 4;
  if (buf.byteLength !== expected) {
    throw new Error(`frame:// body is ${buf.byteLength} bytes, expected ${expected}`);
  }
  // A view over the fetched buffer — no copy.
  return { width, height, scale, data: new Uint8ClampedArray(buf) as Uint8ClampedArray<ArrayBuffer> };
}

/**
 * Fetch and paint onto `canvas` (self-test lazy-fetch proof; the Windows fallback path
 * where no native frame layer exists). The bitmap is sized to physical pixels and the CSS
 * box to points so one bitmap pixel maps to one device pixel.
 */
export async function loadFrame(displayId: number, gen: number, token: string, canvas: HTMLCanvasElement): Promise<FrameInfo> {
  const frame = await fetchFrame(displayId, gen, token);
  paintFrame(canvas, frame);
  return { width: frame.width, height: frame.height, scale: frame.scale };
}

/** Paint fetched RGBA onto `canvas` (bitmap in physical px, CSS box in points). */
export function paintFrame(canvas: HTMLCanvasElement, frame: FrameRgba & FrameInfo): void {
  canvas.width = frame.width;
  canvas.height = frame.height;
  canvas.style.width = `${frame.width / frame.scale}px`;
  canvas.style.height = `${frame.height / frame.scale}px`;
  const ctx = canvas.getContext("2d");
  if (!ctx) {
    throw new Error("2D context unavailable on the frame canvas");
  }
  ctx.putImageData(new ImageData(frame.data as Uint8ClampedArray<ArrayBuffer>, frame.width, frame.height), 0, 0);
}
