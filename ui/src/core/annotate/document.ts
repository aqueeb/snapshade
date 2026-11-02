// AnnotationDocument — the vector shape list with snapshot undo
// (D-02; ARCHITECTURE Pattern 5 "Vector annotations, snapshot undo, rasterize on export").
//
// Shape coordinates are SELECTION-LOCAL physical pixels (origin = the selection's
// top-left), so the document is independent of where the selection sits and
// `render()` can draw it at (0,0) of the crop for both preview and export (D-03).
import type { PhysicalPoint, PhysicalRect } from "../geometry";
import { UndoStack } from "./undo";

/** One annotation. `color` is a CSS colour string; `size` the stroke size in px. */
export type Shape =
  | { kind: "pen"; pts: PhysicalPoint[]; color: string; size: number }
  | { kind: "line" | "arrow"; a: PhysicalPoint; b: PhysicalPoint; color: string; size: number }
  | { kind: "rect" | "ellipse"; r: PhysicalRect; color: string; size: number }
  | { kind: "marker"; pts: PhysicalPoint[]; color: string; size: number }
  | { kind: "blur"; r: PhysicalRect; block: number }
  | { kind: "text"; at: PhysicalPoint; text: string; color: string; size: number };

export class AnnotationDocument {
  private items: Shape[] = [];
  /** Undo history (docs/SPEC.md §4): snapshots of `items`. */
  private readonly history = new UndoStack<Shape[]>();

  /** Shapes in commit order (oldest first); the array is not to be mutated. */
  get shapes(): readonly Shape[] {
    return this.items;
  }

  /** Append a finished shape, saving a structural copy of the previous list for undo. */
  commit(s: Shape): void {
    this.history.push(cloneShapes(this.items));
    this.items = [...this.items, cloneShape(s)];
  }

  /** Remove the last annotation (spec §4 Undo; ⌘Z). Returns false when nothing to undo. */
  undo(): boolean {
    const prev = this.history.pop();
    if (prev === undefined) return false;
    this.items = prev;
    return true;
  }

  canUndo(): boolean {
    return this.history.length > 0;
  }

  /**
   * Drop every shape and the undo history. Called when a NEW selection starts
   * (click-outside restart or a drag on another display): shapes are
   * selection-local, so they cannot outlive the rect they were drawn on.
   */
  clear(): void {
    this.items = [];
    while (this.history.pop() !== undefined) {
      // drain
    }
  }
}

function cloneShape(s: Shape): Shape {
  return structuredClone(s);
}

function cloneShapes(list: readonly Shape[]): Shape[] {
  return list.map(cloneShape);
}
