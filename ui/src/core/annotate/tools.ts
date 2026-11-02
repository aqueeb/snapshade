// ToolState — the active drawing tool, the shared colour, and the stroke size
// (D-02; ARCHITECTURE §State Management `tool: ToolState {active, color, size, marker_color}`).
//
// ONE width and ONE colour are shared by every tool and every screen in a capture
// (docs/SPEC.md §4). The marker is the one tool with its own
// default colour (spec §4: "Marker … default color yellow"; Color: "Default red").

/** The eight editing tools of spec §4, in toolbar order (Blur per D-25, Windows feature set). */
export type ToolId = "pen" | "line" | "arrow" | "rect" | "ellipse" | "marker" | "blur" | "text";

export class ToolState {
  /** Tool currently armed on the vertical toolbar; `null` = plain selection mode. */
  active: ToolId | null = null;
  /** Shared colour for pen/line/arrow/rect/ellipse/text (spec §4 Color: default red). */
  color = "#ff0000";
  /** Marker colour (spec §4 Marker: default yellow). */
  markerColor = "#ffff00";
  /**
   * Stroke size in px, changed by the mouse wheel (spec §4: wheel down = bigger,
   * wheel up = smaller, with min/max limits): step 1, range [1, 40], initial 3.
   */
  size = 3;
  readonly min = 1;
  readonly max = 40;

  /** Change the size by `delta` (sign = wheel direction), clamped to `[min, max]`; returns the new size. */
  adjustSize(delta: number): number {
    this.size = Math.min(this.max, Math.max(this.min, this.size + delta));
    return this.size;
  }

  /** The colour a tool draws with: the marker has its own, everything else shares `color`. */
  colorFor(tool: ToolId): string {
    return tool === "marker" ? this.markerColor : this.color;
  }
}
