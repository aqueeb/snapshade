// Zero-copy frame handoff on Windows (Plan 999.4-01): Rust posts one WebView2 shared buffer
// per overlay window to this page (`chrome.webview` `sharedbufferreceived`, in reply to
// `overlay_ready`); every capture writes the display's RGBA into that memory and
// `frame-ready` (`fmt: "shared"`) says "paint from it". The ArrayBuffer is a view of the
// shared memory — no fetch, no copy — and the same bytes feed the annotation preview and
// the export. No Tauri import here, like `frame.ts`: the module is host-agnostic and the
// receiver takes the bridge object, so Vitest drives it with a fake.
import type { FrameRgba } from "../core/render/canvas";
import type { FrameInfo } from "./frame";

/** `event.additionalData` of the post (Rust `SharedBufferInfo`, camelCase on the wire). */
export interface SharedBufferInfo {
  displayId: number;
  width: number;
  height: number;
  scale: number;
  epoch: number;
}

export interface SharedFrame {
  info: SharedBufferInfo;
  buffer: ArrayBuffer;
}

/** The WebView2 event: `getBuffer()` yields the ArrayBuffer, `additionalData` the parsed JSON. */
export interface SharedBufferReceivedEvent {
  getBuffer(): ArrayBuffer;
  additionalData: unknown;
}

/** The subset of `window.chrome.webview` this module uses. */
export interface WebView2Bridge {
  addEventListener(type: "sharedbufferreceived", listener: (event: SharedBufferReceivedEvent) => void): void;
  releaseBuffer(buffer: ArrayBuffer): void;
}

declare global {
  interface Window {
    chrome?: { webview?: WebView2Bridge };
  }
}

function positiveInt(v: unknown): v is number {
  return typeof v === "number" && Number.isInteger(v) && v > 0;
}

/** Validate `additionalData`; `null` for anything that is not a well-formed `SharedBufferInfo`. */
export function parseSharedBufferInfo(data: unknown): SharedBufferInfo | null {
  if (typeof data !== "object" || data === null) return null;
  const d = data as Record<string, unknown>;
  const { displayId, width, height, scale, epoch } = d;
  if (!positiveInt(displayId) || !positiveInt(width) || !positiveInt(height)) return null;
  if (typeof scale !== "number" || !Number.isFinite(scale) || scale <= 0) return null;
  if (typeof epoch !== "number" || !Number.isInteger(epoch) || epoch < 0) return null;
  return { displayId, width, height, scale, epoch };
}

/** What a `frame-ready` payload asks the buffer to be. */
export interface WantedFrame {
  displayId: number;
  width: number;
  height: number;
  scale: number;
}

/**
 * The RGBA view a `frame-ready` payload may paint from, or the reason it cannot (a string;
 * the caller falls back to the fetch path). The view is `width * height * 4` bytes from the
 * start of the buffer; a detached buffer has `byteLength` 0 and fails the length check.
 */
export function sharedFrameView(frame: SharedFrame | null, want: WantedFrame): (FrameRgba & FrameInfo) | string {
  if (!frame) return "no shared buffer";
  const { info, buffer } = frame;
  if (info.displayId !== want.displayId) return `buffer is for display ${info.displayId}, frame is for ${want.displayId}`;
  if (info.width !== want.width || info.height !== want.height) {
    return `buffer is ${info.width}x${info.height}, frame is ${want.width}x${want.height}`;
  }
  const bytes = want.width * want.height * 4;
  if (buffer.byteLength < bytes) return `buffer holds ${buffer.byteLength} bytes, frame needs ${bytes}`;
  return { width: want.width, height: want.height, scale: want.scale, data: new Uint8ClampedArray(buffer, 0, bytes) };
}

/**
 * Holds the page's shared buffer. Construct it BEFORE invoking `overlay_ready` (the post
 * answers that command); `wait()` covers a `frame-ready` that lands before the event.
 */
export class SharedFrameReceiver {
  /** `false` outside WebView2 (no `chrome.webview`): `wait()` resolves `null` at once. */
  readonly available: boolean;
  private frame: SharedFrame | null = null;
  private waiters: ((frame: SharedFrame | null) => void)[] = [];

  constructor(
    bridge: WebView2Bridge | undefined = typeof window === "undefined" ? undefined : window.chrome?.webview,
    private readonly log: (message: string) => void = (message) => console.log(message),
  ) {
    this.available = bridge !== undefined;
    bridge?.addEventListener("sharedbufferreceived", (event) => this.receive(event, bridge));
  }

  /** The current buffer, if one has arrived. */
  current(): SharedFrame | null {
    return this.frame;
  }

  /** Resolves with the buffer (at once when present), or `null` after `timeoutMs`. */
  wait(timeoutMs: number): Promise<SharedFrame | null> {
    if (this.frame || !this.available) return Promise.resolve(this.frame);
    return new Promise((resolve) => {
      const waiter = (frame: SharedFrame | null): void => {
        clearTimeout(timer);
        resolve(frame);
      };
      const timer = setTimeout(() => {
        this.waiters = this.waiters.filter((w) => w !== waiter);
        resolve(null);
      }, timeoutMs);
      this.waiters.push(waiter);
    });
  }

  private receive(event: SharedBufferReceivedEvent, bridge: WebView2Bridge): void {
    const info = parseSharedBufferInfo(event.additionalData);
    if (!info) {
      this.log(`shared frame buffer ignored: malformed additionalData ${JSON.stringify(event.additionalData)}`);
      return;
    }
    let buffer: ArrayBuffer;
    try {
      buffer = event.getBuffer();
    } catch (err: unknown) {
      this.log(`shared frame buffer ignored: getBuffer failed (${String(err)})`);
      return;
    }
    if (this.frame) {
      // A re-post (new buffer after a monitor size change): let the old mapping go.
      try {
        bridge.releaseBuffer(this.frame.buffer);
      } catch {
        /* already released */
      }
    }
    this.frame = { info, buffer };
    this.log(`shared frame buffer received: display ${info.displayId} ${info.width}x${info.height} (${buffer.byteLength} bytes)`);
    const waiters = this.waiters;
    this.waiters = [];
    for (const waiter of waiters) waiter(this.frame);
  }
}
