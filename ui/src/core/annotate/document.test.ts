// AnnotationDocument + ToolState (Plan 01-05 Task 2, Tests 1-2).
// Sources: ARCHITECTURE Pattern 5 (vector shapes, snapshot undo);
// docs/SPEC.md §4 (snapshot undo without redo; one size and one colour shared across
// tools; the defaults).
import { describe, expect, it } from "vitest";
import { AnnotationDocument, type Shape } from "./document";
import { ToolState } from "./tools";
import { UndoStack } from "./undo";

const pen = (n: number): Extract<Shape, { kind: "pen" }> => ({ kind: "pen", pts: [{ x: n, y: n }], color: "#ff0000", size: 3 });

describe("AnnotationDocument snapshot undo (ANNOT-11; _undoStack, no redo)", () => {
  it("commit appends shapes in order", () => {
    const doc = new AnnotationDocument();
    expect(doc.shapes).toEqual([]);
    doc.commit(pen(1));
    doc.commit(pen(2));
    expect(doc.shapes.map((s) => (s.kind === "pen" ? s.pts[0]!.x : -1))).toEqual([1, 2]);
  });

  it("undo restores the previous snapshot and returns true", () => {
    const doc = new AnnotationDocument();
    doc.commit(pen(1));
    doc.commit(pen(2));
    expect(doc.canUndo()).toBe(true);
    expect(doc.undo()).toBe(true);
    expect(doc.shapes.length).toBe(1);
    expect(doc.shapes[0]).toEqual(pen(1));
  });

  it("undo on an empty stack returns false", () => {
    const doc = new AnnotationDocument();
    expect(doc.canUndo()).toBe(false);
    expect(doc.undo()).toBe(false);
    expect(doc.shapes).toEqual([]);
  });

  it("after 3 commits and 2 undos one shape remains", () => {
    const doc = new AnnotationDocument();
    doc.commit(pen(1));
    doc.commit(pen(2));
    doc.commit(pen(3));
    doc.undo();
    doc.undo();
    expect(doc.shapes.length).toBe(1);
    expect(doc.undo()).toBe(true);
    expect(doc.shapes.length).toBe(0);
    expect(doc.undo()).toBe(false);
  });

  it("snapshots are structural copies: mutating a committed shape later does not leak into undo", () => {
    const doc = new AnnotationDocument();
    const first = pen(1);
    doc.commit(first);
    doc.commit(pen(2));
    first.pts[0]!.x = 99;
    doc.undo();
    expect(doc.shapes[0]).toEqual(pen(1));
  });

  it("has no redo (EDIT-02 is v2)", () => {
    const doc = new AnnotationDocument();
    expect((doc as unknown as Record<string, unknown>).redo).toBeUndefined();
    const stack = new UndoStack<number>();
    expect((stack as unknown as Record<string, unknown>).redo).toBeUndefined();
  });

  it("UndoStack is LIFO and reports its length", () => {
    const stack = new UndoStack<number>();
    expect(stack.length).toBe(0);
    stack.push(1);
    stack.push(2);
    expect(stack.length).toBe(2);
    expect(stack.pop()).toBe(2);
    expect(stack.pop()).toBe(1);
    expect(stack.pop()).toBeUndefined();
    expect(stack.length).toBe(0);
  });
});

describe("ToolState defaults and size clamps (spec §4; _currentWidth / _currentColor global)", () => {
  it("defaults: red, marker yellow, no active tool, size 3", () => {
    const t = new ToolState();
    expect(t.color).toBe("#ff0000");
    expect(t.markerColor).toBe("#ffff00");
    expect(t.active).toBeNull();
    expect(t.size).toBe(3);
    expect(t.min).toBe(1);
    expect(t.max).toBe(40);
  });

  it("colorFor: marker uses markerColor, every other tool the shared colour", () => {
    const t = new ToolState();
    expect(t.colorFor("marker")).toBe(t.markerColor);
    expect(t.colorFor("pen")).toBe(t.color);
    expect(t.colorFor("arrow")).toBe(t.color);
    t.color = "#00ff00";
    expect(t.colorFor("text")).toBe("#00ff00");
    expect(t.colorFor("marker")).toBe("#ffff00");
  });

  it("adjustSize(+1) increments until max then stays", () => {
    const t = new ToolState();
    expect(t.adjustSize(1)).toBe(4);
    for (let i = 0; i < 100; i += 1) t.adjustSize(1);
    expect(t.size).toBe(t.max);
    expect(t.adjustSize(1)).toBe(t.max);
  });

  it("adjustSize(-1) decrements until min then stays", () => {
    const t = new ToolState();
    expect(t.adjustSize(-1)).toBe(2);
    for (let i = 0; i < 100; i += 1) t.adjustSize(-1);
    expect(t.size).toBe(t.min);
    expect(t.adjustSize(-1)).toBe(t.min);
  });
});
