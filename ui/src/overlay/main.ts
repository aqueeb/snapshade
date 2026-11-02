// Overlay page bootstrap (Plan 01-06/01-07/01-09; rewritten for the real selection and
// annotation UI in Phases 2–3).
//
// One hidden overlay window exists per display, labelled `overlay-<display_id>` (or
// `overlay-<display_id>-<epoch>` after a display-change re-prepare). On macOS the frozen
// frame is NOT painted by this page: Rust installs each display's CGImage on a native
// CALayer beneath this transparent webview before the panel is shown. The page is
// pre-dimmed at load so the first composited frame is already dimmed; on `frame-ready`
// (metadata only, after show) it re-paints the dim, reports `overlay_painted` after a
// double rAF (D-09 point 4) and ONLY THEN fetches the frame bytes over `frame://` for the
// annotation preview and export (off the latency path). On Windows there is no native
// layer and the page paints the frame onto `#frame` first: `fmt: "shared"` (Plan 999.4-01)
// paints from the WebView2 shared buffer Rust posted to this page at `overlay_ready` and
// wrote the pixels into — no transfer, the same bytes serve preview and export; `fmt:
// "fetch"` fetches the frame over `frame://` (the fallback, and the 999.3 behaviour).
//
// Spike instrumentation kept: every key is reported over `overlay_key` as a NAME (printable
// keys as `<printable>`, T-01-18); the `#probe` input is shown only for self-test runs.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { PhysicalRect } from "../core/geometry";
import type { FrameRgba } from "../core/render/canvas";
import { OverlayChrome } from "./chrome";
import { OverlayController } from "./controller";
import { fetchFrame, paintFrame } from "./frame";
import { paintDim, type DimGeometry } from "./paint";
import { SharedFrameReceiver, sharedFrameView } from "./shared_frame";
import { t } from "../strings";

/** How long a `fmt: "shared"` paint waits for the buffer before falling back to a fetch. */
const SHARED_BUFFER_WAIT_MS = 1000;

/** Payload of the `frame-ready` event (Rust `FrameReady`, camelCase on the wire). */
interface FrameReady {
  gen: number;
  displayId: number;
  width: number;
  height: number;
  scale: number;
  /**
   * `"native"` (macOS: frame on the CALayer), `"shared"` (Windows: paint from this page's
   * shared buffer) or `"fetch"` (fetch over `frame://` and paint it here first).
   */
  fmt: string;
  token: string;
  fetchAfterPaint: boolean;
  initialSelection: PhysicalRect | null;
  tool: { color: string; markerColor: string; size: number };
}

interface KeyEquivalent {
  key: string;
  meta: boolean;
  shift: boolean;
}

interface HomeChanged {
  displayId: number;
}

function requireEl<T extends HTMLElement>(id: string, ctor: new () => T): T {
  const el = document.getElementById(id);
  if (!(el instanceof ctor)) throw new Error(`overlay: #${id} missing or wrong type`);
  return el;
}

function displayIdFromLabel(label: string): number {
  const match = /^overlay-(\d+)(?:-\d+)?$/.exec(label);
  if (!match || match[1] === undefined) throw new Error(`overlay: unexpected window label ${label}`);
  return Number(match[1]);
}

function geometryFromWindow(): DimGeometry {
  const scale = window.devicePixelRatio || 1;
  return { width: Math.round(window.innerWidth * scale), height: Math.round(window.innerHeight * scale), scale };
}

function logInvokeError(command: string): (err: unknown) => void {
  return (err: unknown) => console.error(`${command} rejected`, err);
}

let controller: OverlayController | null = null;
let currentGen = 0;
let dimGeometry: DimGeometry = { width: 0, height: 0, scale: 1 };
/** This page's shared frame buffer, once Rust has posted it (Windows; inert elsewhere). */
let sharedFrames: SharedFrameReceiver | null = null;

function showError(message: string): void {
  const el = document.getElementById("error");
  if (el) {
    el.textContent = message;
    el.hidden = false;
  }
}

function clearError(): void {
  const el = document.getElementById("error");
  if (el) el.hidden = true;
}

function onOverlayHidden(displayId: number, dim: HTMLCanvasElement, frameCanvas: HTMLCanvasElement): void {
  controller?.dispose();
  controller = null;
  currentGen = 0;
  frameCanvas.width = 0;
  frameCanvas.height = 0;
  frameCanvas.hidden = true;
  clearError();
  paintDim(dim, dimGeometry);
  document.body.style.cursor = "crosshair";
  console.log(`overlay ${displayId}: hidden, dim reset`);
}

function installKeyReporting(displayId: number, probe: HTMLInputElement): void {
  window.addEventListener("keydown", (event: KeyboardEvent) => {
    const inInput = document.activeElement === probe || document.activeElement instanceof HTMLTextAreaElement;
    if (inInput && event.key.length === 1) return;
    const name = event.key.length === 1 && !(event.metaKey || event.ctrlKey) ? "<printable>" : event.key;
    void invoke("overlay_key", { displayId, key: name, meta: event.metaKey, shift: event.shiftKey }).catch(logInvokeError("overlay_key"));
  });
  probe.addEventListener("input", () => {
    void invoke("overlay_input", { displayId, len: probe.value.length }).catch(logInvokeError("overlay_input"));
  });
}

async function main(): Promise<void> {
  const displayId = displayIdFromLabel(getCurrentWindow().label);
  console.log(`overlay page loaded for display ${displayId}`);
  const dim = requireEl("dim", HTMLCanvasElement);
  const frameCanvas = requireEl("frame", HTMLCanvasElement);
  const anno = requireEl("anno", HTMLCanvasElement);
  const textbox = requireEl("textbox", HTMLTextAreaElement);
  const colorInput = requireEl("color", HTMLInputElement);
  const probe = requireEl("probe", HTMLInputElement);
  const root = requireEl("chrome", HTMLDivElement);

  // Pre-dim before anything else: the first composited frame after show must already be
  // dimmed, with no event round-trip in between (Plan B).
  dimGeometry = geometryFromWindow();
  paintDim(dim, dimGeometry);
  installKeyReporting(displayId, probe);
  // The listener must exist before `overlay_ready` below: that command posts the buffer.
  sharedFrames = new SharedFrameReceiver();
  console.log(`overlay ${displayId}: shared frame buffers ${sharedFrames.available ? "available" : "not available (no chrome.webview)"}`);

  // Chrome is built once; handlers route to whichever controller is live.
  const chrome = new OverlayChrome(root, {
    onTool: (tool) => controller?.onTool(tool),
    onColorButton: (anchor) => controller?.onColorButton(anchor),
    onUndo: () => controller?.onUndo(),
    onAction: (action) => controller?.onAction(action),
    onPalette: (color) => controller?.onPalette(color),
    onCustomColor: () => controller?.onCustomColor(),
    onContextMenu: (action) => controller?.onAction(action),
  });

  await listen("overlay-hidden", () => {
    try {
      onOverlayHidden(displayId, dim, frameCanvas);
    } catch (err: unknown) {
      console.error("overlay-hidden handling failed", err);
    }
  });
  await listen<HomeChanged>("overlay-home-changed", (event) => {
    if (event.payload.displayId !== displayId) controller?.homeMovedElsewhere();
  });
  // AppKit resolves ⌘-key equivalents before WebKit creates a DOM keydown; Rust forwards
  // them here (Plan 01-10). Reported over `overlay_key` too so the probe line is identical.
  await listen<KeyEquivalent>("overlay-key-equivalent", (event) => {
    const { key, meta, shift } = event.payload;
    void invoke("overlay_key", { displayId, key, meta, shift }).catch(logInvokeError("overlay_key"));
    controller?.keyEquivalent(key, shift);
  });
  await listen<FrameReady>("frame-ready", (event) => {
    const payload = event.payload;
    if (payload.displayId !== displayId) return;
    try {
      onFrameReady(displayId, payload, { dim, frameCanvas, anno, textbox, colorInput, probe, chrome });
    } catch (err: unknown) {
      console.error("frame-ready handling failed", err);
      showError(t("overlay.frameFailed"));
    }
  });
  // Handshake: the listeners are attached. If a capture is already showing on this
  // display (this page loaded after the event — slow machine, renderer restart), Rust
  // replays its payload here instead of leaving a black overlay (Windows; null elsewhere).
  const pending = await invoke<FrameReady | null>("overlay_ready", { displayId }).catch((err: unknown) => {
    logInvokeError("overlay_ready")(err);
    return null;
  });
  if (pending && pending.displayId === displayId) {
    onFrameReady(displayId, pending, { dim, frameCanvas, anno, textbox, colorInput, probe, chrome });
  }
}

interface Els {
  dim: HTMLCanvasElement;
  frameCanvas: HTMLCanvasElement;
  anno: HTMLCanvasElement;
  textbox: HTMLTextAreaElement;
  colorInput: HTMLInputElement;
  probe: HTMLInputElement;
  chrome: OverlayChrome;
}

function onFrameReady(displayId: number, payload: FrameReady, els: Els): void {
  controller?.dispose();
  controller = null;
  clearError();
  dimGeometry = { width: payload.width, height: payload.height, scale: payload.scale };
  currentGen = payload.gen;
  els.probe.hidden = !payload.fetchAfterPaint;
  paintDim(els.dim, dimGeometry);

  let resolveFrame!: (f: FrameRgba) => void;
  let rejectFrame!: (e: unknown) => void;
  const framePromise = new Promise<FrameRgba>((res, rej) => {
    resolveFrame = res;
    rejectFrame = rej;
  });
  framePromise.catch(() => {
    /* reported by the controller */
  });

  const start = (): void => {
    controller = new OverlayController({
      displayId,
      gen: payload.gen,
      geometry: dimGeometry,
      frame: framePromise,
      dim: els.dim,
      anno: els.anno,
      textbox: els.textbox,
      colorInput: els.colorInput,
      chrome: els.chrome,
      initialSelection: payload.initialSelection,
      tool: payload.tool,
    });
  };

  // Set once the fetch path has handed its bytes to `framePromise`: no second fetch.
  let frameSettled = false;
  const fetchPixels = async (): Promise<void> => {
    if (frameSettled) return;
    try {
      const frame = await fetchFrame(displayId, payload.gen, payload.token);
      if (currentGen !== payload.gen) return;
      resolveFrame(frame);
      if (payload.fetchAfterPaint) {
        void invoke("overlay_frame_fetched", { displayId, gen: payload.gen, bytes: frame.width * frame.height * 4, error: null }).catch(
          logInvokeError("overlay_frame_fetched"),
        );
      }
    } catch (err: unknown) {
      rejectFrame(err);
      showError(t("overlay.frameFailed"));
      if (payload.fetchAfterPaint) {
        void invoke("overlay_frame_fetched", { displayId, gen: payload.gen, bytes: 0, error: String(err) }).catch(logInvokeError("overlay_frame_fetched"));
      }
    }
  };

  const reportPainted = (): void => {
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        if (currentGen !== payload.gen) return; // hidden before presented: never report
        void invoke("overlay_painted", { displayId, gen: payload.gen, tWallMs: performance.timeOrigin + performance.now() }).catch(
          logInvokeError("overlay_painted"),
        );
        // Ordering invariant (D-09 point 4): the fetch starts only after the paint report.
        void fetchPixels();
      });
    });
  };

  // Windows: the page paints the frame itself. Both branches end the same way — the painted
  // bytes feed `framePromise` (annotation preview / export), so nothing is fetched twice —
  // and both report their durations to Rust (`frame fetched …` on stderr): the page console
  // is not visible on a Windows run. `tStart` is when `frame-ready` arrived.
  const tStart = performance.now();
  const painted = (frame: FrameRgba & { scale: number }, readyMs: number): void => {
    const tReady = performance.now();
    paintFrame(els.frameCanvas, frame);
    const tPainted = performance.now();
    void invoke("overlay_frame_fetched", {
      displayId,
      gen: payload.gen,
      bytes: frame.width * frame.height * 4,
      error: null,
      fetchMs: readyMs,
      paintMs: tPainted - tReady,
    }).catch(logInvokeError("overlay_frame_fetched"));
    if (currentGen !== payload.gen) return;
    frameSettled = true;
    resolveFrame(frame);
    start();
    reportPainted();
  };
  const fetchAndPaint = (): void => {
    void fetchFrame(displayId, payload.gen, payload.token).then(
      (frame) => painted(frame, performance.now() - tStart),
      (err: unknown) => {
        console.error("frame paint failed", err);
        void invoke("overlay_frame_fetched", { displayId, gen: payload.gen, bytes: 0, error: String(err) }).catch(logInvokeError("overlay_frame_fetched"));
        rejectFrame(err);
        showError(t("overlay.frameFailed"));
      },
    );
  };

  if (payload.fmt === "shared") {
    // Plan 999.4-01: the pixels are already in this page's shared buffer; paint a view of
    // it (`fetchMs` is only the wait for the buffer, normally 0). If it has not arrived —
    // a runtime without shared buffers, or a hotkey within ms of this page's load — the
    // display falls back to the fetch path below.
    els.frameCanvas.hidden = false;
    const receiver = sharedFrames;
    void (receiver ? receiver.wait(SHARED_BUFFER_WAIT_MS) : Promise.resolve(null)).then((shared) => {
      if (currentGen !== payload.gen) return;
      const view = sharedFrameView(shared, payload);
      if (typeof view === "string") {
        console.warn(`overlay ${displayId}: shared frame unavailable (${view}); fetching over frame://`);
        fetchAndPaint();
        return;
      }
      painted(view, performance.now() - tStart);
    });
    return;
  }
  if (payload.fmt === "fetch") {
    els.frameCanvas.hidden = false;
    fetchAndPaint();
    return;
  }
  els.frameCanvas.hidden = true;
  start();
  reportPainted();
}

main().catch((err: unknown) => {
  console.error("overlay bootstrap failed", err);
});
