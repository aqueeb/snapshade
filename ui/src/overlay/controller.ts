// OverlayController — one per display panel per capture generation. Turns DOM events into
// `OverlayInput`, feeds the pure `CaptureSession`, and renders its events: the dim/hole/
// handles layer, the annotation preview (`render()` crop + shapes), the toolbars, the text
// box (IME-hosted, ANNOT-08), the palette, and the four export paths through `export_image`.
//
// Each panel is its own JS context, so cross-display rules are coordinated through Rust:
// a press here claims the selection (`overlay_claim` → `overlay-home-changed` to every
// panel) and the others reset. The session therefore holds a one-display topology.
import { invoke } from "@tauri-apps/api/core";
import type { Shape } from "../core/annotate/document";
import { ToolState, type ToolId } from "../core/annotate/tools";
import { newTopology, roundHalfUp, type DisplayInfo, type PhysicalPoint, type PhysicalRect } from "../core/geometry";
import type { FrameRgba } from "../core/render/canvas";
import { render, textFont, textFontPx, TEXT_LINE_HEIGHT } from "../core/render/render";
import type { Mods } from "../core/selection/model";
import { CaptureSession, type SessionAction, type SessionEvent } from "../core/session";
import { OverlayChrome } from "./chrome";
import { paintDim, paintDimWithHole, type DimGeometry } from "./paint";

export interface ControllerHost {
  displayId: number;
  gen: number;
  geometry: DimGeometry;
  /** The frozen frame, resolved after `overlay_painted` was reported. */
  frame: Promise<FrameRgba>;
  dim: HTMLCanvasElement;
  anno: HTMLCanvasElement;
  textbox: HTMLTextAreaElement;
  colorInput: HTMLInputElement;
  chrome: OverlayChrome;
  initialSelection: PhysicalRect | null;
  tool: { color: string; markerColor: string; size: number };
}

const HINT_KEY = "snapshade.sizeHintShown";

function mods(e: { metaKey: boolean; ctrlKey: boolean; shiftKey: boolean; altKey: boolean }): Mods {
  return { meta: e.metaKey, ctrl: e.ctrlKey, shift: e.shiftKey, alt: e.altKey };
}

function logInvokeError(command: string): (err: unknown) => void {
  return (err: unknown) => console.error(`${command} rejected`, err);
}

const CURSOR: Record<string, string> = {
  n: "ns-resize",
  s: "ns-resize",
  e: "ew-resize",
  w: "ew-resize",
  ne: "nesw-resize",
  sw: "nesw-resize",
  nw: "nwse-resize",
  se: "nwse-resize",
  body: "move",
  outside: "crosshair",
};

export class OverlayController {
  private readonly h: ControllerHost;
  private readonly session: CaptureSession;
  private readonly info: DisplayInfo;
  private frame: FrameRgba | null = null;
  private pointerId: number | null = null;
  private textAt: PhysicalPoint | null = null;
  private lastReported: PhysicalRect | null = null;
  private busy = false;
  private readonly off: (() => void)[] = [];
  private toolPersist: number | null = null;

  constructor(host: ControllerHost) {
    this.h = host;
    const { width, height, scale } = host.geometry;
    this.info = {
      id: host.displayId,
      logical: { x: 0, y: 0, w: width / scale, h: height / scale },
      scale,
      physical: { w: width, h: height },
      primary: true,
    };
    const tool = new ToolState();
    tool.color = host.tool.color;
    tool.markerColor = host.tool.markerColor;
    tool.size = Math.min(tool.max, Math.max(tool.min, host.tool.size));
    this.session = new CaptureSession(newTopology([this.info]), new Map(), tool);
    this.h.chrome.setColor(tool.color);
    this.h.chrome.setActiveTool(null);
    this.h.chrome.setCanUndo(false);
    this.h.anno.hidden = true;
    this.h.textbox.hidden = true;
    paintDim(this.h.dim, host.geometry);
    this.attach();
    void host.frame.then(
      (f) => {
        this.frame = f;
        this.repaint();
      },
      (err: unknown) => console.error("frame fetch failed", err),
    );
    if (host.initialSelection) {
      this.apply(this.session.handle({ t: "restore", display: host.displayId, rect: host.initialSelection }));
    }
  }

  /** Stop handling events and clear every visual (hide / new generation). */
  dispose(): void {
    for (const f of this.off) f();
    this.off.length = 0;
    this.h.chrome.reset();
    this.h.anno.hidden = true;
    this.h.textbox.hidden = true;
    this.frame = null;
    paintDim(this.h.dim, this.h.geometry);
  }

  /** Another display took the selection: drop ours (hideAllSelections). */
  homeMovedElsewhere(): void {
    this.cancelText();
    this.apply(this.session.handle({ t: "action", action: "clear" }));
  }

  /** A ⌘-key AppKit consumed (`overlay-key-equivalent`): same path as a DOM keydown. */
  keyEquivalent(key: string, shift: boolean): void {
    if (!this.h.textbox.hidden && (key === "a" || key === "c" || key === "x")) return; // editing text
    this.apply(this.session.handle({ t: "key", key, mods: { meta: true, ctrl: false, shift, alt: false }, display: this.h.displayId }));
  }

  // --- wiring ---------------------------------------------------------------------------

  private listen<K extends keyof HTMLElementEventMap>(el: HTMLElement | Window, type: K, fn: (e: HTMLElementEventMap[K]) => void, opts?: AddEventListenerOptions): void {
    el.addEventListener(type, fn as EventListener, opts);
    this.off.push(() => el.removeEventListener(type, fn as EventListener, opts));
  }

  private attach(): void {
    const dim = this.h.dim;
    this.listen(dim, "pointerdown", (e) => this.onPointerDown(e));
    this.listen(dim, "pointermove", (e) => this.onPointerMove(e));
    this.listen(dim, "pointerup", (e) => this.onPointerUp(e));
    this.listen(dim, "pointercancel", (e) => this.onPointerUp(e));
    this.listen(dim, "dblclick", (e) => {
      e.preventDefault();
      this.apply(this.session.handle({ t: "action", action: "fullscreen", display: this.h.displayId }));
    });
    this.listen(dim, "contextmenu", (e) => {
      e.preventDefault();
      this.h.chrome.showMenu(e.clientX, e.clientY);
    });
    this.listen(window, "wheel", (e) => this.onWheel(e), { passive: false });
    this.listen(window, "keydown", (e) => this.onKeyDown(e));
    this.listen(this.h.textbox, "keydown", (e) => this.onTextKey(e));
    this.listen(this.h.textbox, "blur", () => this.commitText());
    this.listen(this.h.textbox, "input", () => this.sizeTextBox());
    this.listen(this.h.colorInput, "input", () => this.setColor(this.h.colorInput.value));
    // Pointer events on the annotation canvas fall through to the dim handler: same mapping.
    this.listen(this.h.anno, "pointerdown", (e) => this.onPointerDown(e));
    this.listen(this.h.anno, "pointermove", (e) => this.onPointerMove(e));
    this.listen(this.h.anno, "pointerup", (e) => this.onPointerUp(e));
    this.listen(this.h.anno, "dblclick", (e) => {
      e.preventDefault();
      this.apply(this.session.handle({ t: "action", action: "fullscreen", display: this.h.displayId }));
    });
    this.listen(this.h.anno, "contextmenu", (e) => {
      e.preventDefault();
      this.h.chrome.showMenu(e.clientX, e.clientY);
    });
  }

  /** Chrome handlers (built once by main.ts, routed to the live controller). */
  onTool(tool: ToolId): void {
    this.cancelText();
    const next = this.session.tool.active === tool ? null : tool;
    this.apply(this.session.handle({ t: "tool", tool: next }));
    this.h.chrome.setActiveTool(next);
    this.h.chrome.hidePalette();
    if (next && localStorage.getItem(HINT_KEY) !== "1") {
      try {
        localStorage.setItem(HINT_KEY, "1");
      } catch {
        /* private mode */
      }
      this.h.chrome.showHint();
    }
    this.updateCursor(null);
  }

  onColorButton(anchor: HTMLElement): void {
    if (this.h.chrome.palette.hidden) this.h.chrome.showPalette(anchor);
    else this.h.chrome.hidePalette();
  }

  onPalette(color: string): void {
    this.h.chrome.hidePalette();
    this.setColor(color);
  }

  onCustomColor(): void {
    this.h.chrome.hidePalette();
    this.h.colorInput.value = this.session.tool.color;
    this.h.colorInput.click(); // WKWebView / WebView2 open the native colour dialog (ANNOT-09)
  }

  onUndo(): void {
    this.cancelText();
    this.apply(this.session.handle({ t: "action", action: "undo" }));
  }

  onAction(action: SessionAction): void {
    this.commitText();
    this.h.chrome.hidePalette();
    this.h.chrome.hideMenu();
    this.apply(this.session.handle({ t: "action", action, display: this.h.displayId }));
  }

  // --- pointer --------------------------------------------------------------------------

  private toPhysical(e: PointerEvent): PhysicalPoint {
    const { width, height, scale } = this.h.geometry;
    return {
      x: Math.min(Math.max(roundHalfUp(e.clientX * scale), 0), width),
      y: Math.min(Math.max(roundHalfUp(e.clientY * scale), 0), height),
    };
  }

  private onPointerDown(e: PointerEvent): void {
    this.h.chrome.hideMenu();
    this.h.chrome.hidePalette();
    if (e.button !== 0) return;
    if (this.busy) return;
    if (!this.h.textbox.hidden) {
      this.commitText();
      return; // click-away commits; the next click acts
    }
    e.preventDefault();
    this.pointerId = e.pointerId;
    this.h.dim.setPointerCapture(e.pointerId);
    if (this.session.homeDisplay === null) {
      void invoke("overlay_claim", { displayId: this.h.displayId }).catch(logInvokeError("overlay_claim"));
    }
    this.h.chrome.hideTip();
    this.apply(this.session.handle({ t: "pointerDown", display: this.h.displayId, p: this.toPhysical(e), mods: mods(e) }));
  }

  private onPointerMove(e: PointerEvent): void {
    const p = this.toPhysical(e);
    if (this.pointerId === null) {
      const sel = this.session.selection();
      if (!sel || !sel.model.rect()) this.h.chrome.moveTip(e.clientX, e.clientY);
      this.updateCursor(p);
      return;
    }
    if (e.pointerId !== this.pointerId) return;
    this.apply(this.session.handle({ t: "pointerMove", display: this.h.displayId, p }));
  }

  private onPointerUp(e: PointerEvent): void {
    if (this.pointerId === null || e.pointerId !== this.pointerId) return;
    this.pointerId = null;
    if (this.h.dim.hasPointerCapture(e.pointerId)) this.h.dim.releasePointerCapture(e.pointerId);
    this.apply(this.session.handle({ t: "pointerUp", display: this.h.displayId, p: this.toPhysical(e) }));
  }

  private updateCursor(p: PhysicalPoint | null): void {
    const sel = this.session.selection();
    let cursor = "crosshair";
    if (sel && p) {
      const hit = sel.model.hitTest(p);
      cursor = CURSOR[hit] ?? "crosshair";
      if (hit === "body" && this.session.tool.active) cursor = this.session.tool.active === "text" ? "text" : "crosshair";
    }
    document.body.style.cursor = cursor;
  }

  private onWheel(e: WheelEvent): void {
    if (!this.h.textbox.hidden) return;
    e.preventDefault();
    this.apply(this.session.handle({ t: "wheel", deltaY: e.deltaY }));
    this.persistToolSoon();
  }

  private onKeyDown(e: KeyboardEvent): void {
    if (document.activeElement === this.h.textbox) return; // handled by onTextKey
    if (e.key === "Escape") {
      e.preventDefault();
      if (!this.h.chrome.palette.hidden || !this.h.chrome.menu.hidden) {
        this.h.chrome.hidePalette();
        this.h.chrome.hideMenu();
        return;
      }
    }
    const events = this.session.handle({ t: "key", key: e.key, mods: mods(e), display: this.h.displayId });
    if (events.length > 0 || e.key.startsWith("Arrow") || e.metaKey || e.ctrlKey) e.preventDefault();
    this.apply(events);
    if ((e.metaKey || e.ctrlKey) && (e.key === "=" || e.key === "+" || e.key === "-")) this.persistToolSoon();
  }

  // --- text tool (ANNOT-08) -------------------------------------------------------------

  private beginText(at: PhysicalPoint): void {
    const sel = this.session.selection()?.model.rect();
    if (!sel) return;
    const { scale } = this.h.geometry;
    const tb = this.h.textbox;
    this.textAt = at;
    tb.value = "";
    tb.hidden = false;
    tb.style.left = `${(sel.x + at.x) / scale}px`;
    tb.style.top = `${(sel.y + at.y) / scale}px`;
    tb.style.font = textFont(this.session.tool.size / scale);
    tb.style.lineHeight = `${TEXT_LINE_HEIGHT}`;
    tb.style.color = this.session.tool.color;
    tb.style.maxWidth = `${Math.max(40, (sel.x + sel.w - (sel.x + at.x)) / scale)}px`;
    this.sizeTextBox();
    tb.focus();
  }

  private sizeTextBox(): void {
    const tb = this.h.textbox;
    tb.style.height = "auto";
    tb.style.width = "auto";
    const lines = tb.value.split("\n");
    const longest = Math.max(4, ...lines.map((l) => l.length));
    tb.style.width = `${Math.min(parseFloat(tb.style.maxWidth) || 400, longest * (textFontPx(this.session.tool.size) / this.h.geometry.scale) * 0.6 + 16)}px`;
    tb.style.height = `${tb.scrollHeight}px`;
  }

  private onTextKey(e: KeyboardEvent): void {
    if (e.key === "Escape") {
      e.preventDefault();
      this.cancelText();
      return;
    }
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      this.commitText();
    }
  }

  private commitText(): void {
    const tb = this.h.textbox;
    if (tb.hidden || !this.textAt) return;
    const text = tb.value;
    const at = this.textAt;
    this.textAt = null;
    tb.hidden = true;
    tb.value = "";
    this.apply(this.session.handle({ t: "commitText", at, text }));
  }

  private cancelText(): void {
    const tb = this.h.textbox;
    if (tb.hidden) return;
    this.textAt = null;
    tb.hidden = true;
    tb.value = "";
  }

  // --- colour / persistence -------------------------------------------------------------

  private setColor(color: string): void {
    if (!/^#[0-9a-fA-F]{6}$/.test(color)) return;
    const tool = this.session.tool;
    if (tool.active === "marker") tool.markerColor = color;
    else tool.color = color;
    this.h.chrome.setColor(tool.active === "marker" ? tool.markerColor : tool.color);
    if (!this.h.textbox.hidden) this.h.textbox.style.color = color;
    this.persistToolSoon();
  }

  private persistToolSoon(): void {
    if (this.toolPersist !== null) window.clearTimeout(this.toolPersist);
    this.toolPersist = window.setTimeout(() => {
      this.toolPersist = null;
      const tool = this.session.tool;
      void invoke("overlay_tool_changed", { color: tool.color, markerColor: tool.markerColor, size: tool.size }).catch(
        logInvokeError("overlay_tool_changed"),
      );
    }, 300);
  }

  // --- session events -------------------------------------------------------------------

  private apply(events: SessionEvent[]): void {
    for (const ev of events) {
      switch (ev.t) {
        case "sceneChanged":
          this.repaint();
          break;
        case "beginText":
          this.beginText(ev.at);
          break;
        case "export":
          void this.exportSelection(ev.kind);
          break;
        case "dismiss":
          this.dismiss();
          break;
      }
    }
  }

  private dismiss(): void {
    this.cancelText();
    void invoke("overlay_cancel").catch(logInvokeError("overlay_cancel"));
  }

  private repaint(): void {
    const sel = this.session.selection();
    const model = sel?.model ?? null;
    const rect = model?.rect() ?? null;
    const { scale } = this.h.geometry;
    const settled = model?.state.kind === "selected";
    paintDimWithHole(this.h.dim, rect, scale, settled);
    this.h.chrome.setCanUndo(this.session.doc.canUndo());
    this.h.chrome.layout(rect, scale, settled);
    if (rect) this.h.chrome.hideTip();
    if (settled && rect && (!this.lastReported || this.lastReported.x !== rect.x || this.lastReported.y !== rect.y || this.lastReported.w !== rect.w || this.lastReported.h !== rect.h)) {
      this.lastReported = rect;
      void invoke("overlay_selection_changed", { displayId: this.h.displayId, rect }).catch(logInvokeError("overlay_selection_changed"));
    }
    this.drawAnnotations(rect);
  }

  /** The annotation preview: crop + shapes onto `#anno` at the selection; hidden when there is nothing to draw. */
  private drawAnnotations(rect: PhysicalRect | null): void {
    const anno = this.h.anno;
    const pending = this.session.pendingShape();
    if (!rect || !this.frame || (this.session.doc.shapes.length === 0 && !pending)) {
      anno.hidden = true;
      return;
    }
    const { scale } = this.h.geometry;
    if (anno.width !== rect.w || anno.height !== rect.h) {
      anno.width = rect.w;
      anno.height = rect.h;
    }
    anno.style.left = `${rect.x / scale}px`;
    anno.style.top = `${rect.y / scale}px`;
    anno.style.width = `${rect.w / scale}px`;
    anno.style.height = `${rect.h / scale}px`;
    const ctx = anno.getContext("2d");
    if (!ctx) return;
    ctx.clearRect(0, 0, rect.w, rect.h);
    render<ImageData>(this.frame, rect, this.session.doc, ctx, pending);
    anno.hidden = false;
  }

  /** Flatten with the ONE render() and hand raw RGBA to Rust (EXPORT-01..05). */
  private async exportSelection(kind: "clipboard" | "save" | "saveAs" | "print"): Promise<void> {
    const rect = this.session.selection()?.model.rect();
    if (!rect || this.busy) return;
    this.busy = true;
    try {
      const frame = this.frame ?? (await this.h.frame);
      const canvas = document.createElement("canvas");
      canvas.width = rect.w;
      canvas.height = rect.h;
      const ctx = canvas.getContext("2d");
      if (!ctx) throw new Error("2D context unavailable for export");
      render<ImageData>(frame, rect, this.session.doc, ctx);
      const pixels = ctx.getImageData(0, 0, rect.w, rect.h).data;
      const body = new Uint8Array(pixels.buffer, pixels.byteOffset, pixels.byteLength);
      const xKind = kind === "clipboard" ? "copy" : kind;
      await invoke("export_image", body, {
        headers: { "x-kind": xKind, "x-width": String(rect.w), "x-height": String(rect.h) },
      });
      // Copy / instant save: the data is on the clipboard / on disk — now close (EXPORT-02).
      // Save As / Print: Rust already hid the panels; this clears the frame store.
      this.dismiss();
    } catch (err: unknown) {
      console.error(`export ${kind} failed`, err);
      this.repaint();
    } finally {
      this.busy = false;
    }
  }
}

/** The in-progress shape type is re-exported for tests of the preview path. */
export type { Shape };
