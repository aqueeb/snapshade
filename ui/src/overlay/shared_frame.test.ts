import { describe, expect, it, vi } from "vitest";
import {
  parseSharedBufferInfo,
  SharedFrameReceiver,
  sharedFrameView,
  type SharedBufferReceivedEvent,
  type WebView2Bridge,
} from "./shared_frame";

const INFO = { displayId: 2, width: 4, height: 3, scale: 1.25, epoch: 0 };

/** A `chrome.webview` stand-in that records the listener and released buffers. */
function fakeBridge(): WebView2Bridge & { fire(event: SharedBufferReceivedEvent): void; released: ArrayBuffer[] } {
  let listener: ((event: SharedBufferReceivedEvent) => void) | null = null;
  const released: ArrayBuffer[] = [];
  return {
    released,
    addEventListener(_type, fn) {
      listener = fn;
    },
    releaseBuffer(buffer) {
      released.push(buffer);
    },
    fire(event) {
      if (!listener) throw new Error("no listener attached");
      listener(event);
    },
  };
}

function event(buffer: ArrayBuffer, additionalData: unknown = INFO): SharedBufferReceivedEvent {
  return { getBuffer: () => buffer, additionalData };
}

describe("parseSharedBufferInfo", () => {
  it("accepts the Rust payload shape and rejects anything malformed", () => {
    expect(parseSharedBufferInfo(INFO)).toEqual(INFO);
    expect(parseSharedBufferInfo({ ...INFO, epoch: 3 })).toEqual({ ...INFO, epoch: 3 });
    for (const bad of [
      null,
      "x",
      {},
      { ...INFO, displayId: 0 },
      { ...INFO, width: 1.5 },
      { ...INFO, height: -1 },
      { ...INFO, scale: 0 },
      { ...INFO, scale: "1" },
      { ...INFO, epoch: -1 },
    ]) {
      expect(parseSharedBufferInfo(bad)).toBeNull();
    }
  });
});

describe("sharedFrameView", () => {
  it("returns a zero-copy view over exactly width*height*4 bytes", () => {
    const buffer = new ArrayBuffer(4 * 3 * 4 + 8); // WebView2 may round the mapping up
    new Uint8Array(buffer)[0] = 200;
    const view = sharedFrameView({ info: INFO, buffer }, INFO);
    expect(typeof view).not.toBe("string");
    if (typeof view === "string") return;
    expect(view.width).toBe(4);
    expect(view.height).toBe(3);
    expect(view.scale).toBe(1.25);
    expect(view.data.byteLength).toBe(48);
    expect(view.data.buffer).toBe(buffer);
    expect(view.data[0]).toBe(200);
  });

  it("names the reason when the buffer does not fit the frame", () => {
    const buffer = new ArrayBuffer(48);
    expect(sharedFrameView(null, INFO)).toBe("no shared buffer");
    expect(sharedFrameView({ info: INFO, buffer }, { ...INFO, displayId: 1 })).toMatch(/display 2, frame is for 1/);
    expect(sharedFrameView({ info: INFO, buffer }, { ...INFO, width: 5 })).toMatch(/4x3, frame is 5x3/);
    expect(sharedFrameView({ info: INFO, buffer: new ArrayBuffer(40) }, INFO)).toMatch(/holds 40 bytes, frame needs 48/);
    // A detached / closed buffer reports byteLength 0 and takes the same branch.
    expect(sharedFrameView({ info: INFO, buffer: new ArrayBuffer(0) }, INFO)).toMatch(/holds 0 bytes/);
  });
});

describe("SharedFrameReceiver", () => {
  it("resolves waiters when the buffer arrives and at once afterwards", async () => {
    vi.useFakeTimers();
    try {
      const bridge = fakeBridge();
      const log: string[] = [];
      const receiver = new SharedFrameReceiver(bridge, (m) => log.push(m));
      expect(receiver.available).toBe(true);
      expect(receiver.current()).toBeNull();
      const pending = receiver.wait(1000);
      const buffer = new ArrayBuffer(48);
      bridge.fire(event(buffer));
      await expect(pending).resolves.toEqual({ info: INFO, buffer });
      await expect(receiver.wait(1)).resolves.toEqual({ info: INFO, buffer });
      expect(log.some((m) => m.includes("display 2 4x3 (48 bytes)"))).toBe(true);
    } finally {
      vi.useRealTimers();
    }
  });

  it("times out to null, ignores malformed posts and releases a replaced buffer", async () => {
    vi.useFakeTimers();
    try {
      const bridge = fakeBridge();
      const receiver = new SharedFrameReceiver(bridge, () => {});
      const pending = receiver.wait(500);
      bridge.fire(event(new ArrayBuffer(48), { nope: true }));
      vi.advanceTimersByTime(500);
      await expect(pending).resolves.toBeNull();
      const first = new ArrayBuffer(48);
      const second = new ArrayBuffer(48);
      bridge.fire(event(first));
      bridge.fire(event(second));
      expect(receiver.current()?.buffer).toBe(second);
      expect(bridge.released).toEqual([first]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("is inert without chrome.webview", async () => {
    const receiver = new SharedFrameReceiver(undefined, () => {});
    expect(receiver.available).toBe(false);
    await expect(receiver.wait(10_000)).resolves.toBeNull();
  });
});
