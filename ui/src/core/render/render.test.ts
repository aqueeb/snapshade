// Golden-image tests for the single render() (D-03, D-04; Plan 01-05 Task 2
// Tests 3-11). Each primitive is rendered onto a 48x32 crop of the deterministic
// 64x48 fixture frame and compared with its committed golden; a few direct pixel
// assertions pin the behaviour the golden encodes (arrowhead present, marker
// translucent, blur blocky) so a wrong golden cannot be rubber-stamped.
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createCanvas, type ImageData as SkiaImageData } from "@napi-rs/canvas";
import { afterAll, describe, expect, expectTypeOf, it } from "vitest";
import { AnnotationDocument, type Shape } from "../annotate/document";
import type { PhysicalRect } from "../geometry";
import type { Canvas2D } from "./canvas";
import { render } from "./render";
import { expectGolden } from "../../../test/golden";
import { makeFrame } from "../../../test/fixture-frame";

const FRAME = makeFrame(64, 48);
const SEL: PhysicalRect = { x: 8, y: 8, w: 48, h: 32 };
const RED = "#ff0000";

function draw(shapes: Shape[]) {
  const canvas = createCanvas(SEL.w, SEL.h);
  const ctx = canvas.getContext("2d");
  const doc = new AnnotationDocument();
  for (const s of shapes) doc.commit(s);
  render<SkiaImageData>(FRAME, SEL, doc, ctx);
  return { canvas, ctx };
}

function px(ctx: ReturnType<typeof draw>["ctx"], x: number, y: number): [number, number, number, number] {
  const d = ctx.getImageData(x, y, 1, 1).data;
  return [d[0]!, d[1]!, d[2]!, d[3]!];
}

function framePx(x: number, y: number): [number, number, number, number] {
  const i = (y * FRAME.width + x) * 4;
  return [FRAME.data[i]!, FRAME.data[i + 1]!, FRAME.data[i + 2]!, FRAME.data[i + 3]!];
}

describe("Canvas2D contract (D-03)", () => {
  it("the browser CanvasRenderingContext2D satisfies Canvas2D without any import in core", () => {
    expectTypeOf<CanvasRenderingContext2D>().toExtend<Canvas2D<ImageData>>();
  });
});

describe("render(): crop", () => {
  it("copies the frame pixels at the selection origin (no shapes)", () => {
    const { ctx } = draw([]);
    // (10,10) in the frame is not on the grid: x%8=2,y%8=2.
    expect(px(ctx, 2, 2)).toEqual(framePx(10, 10));
    // (8,8) is on the grid: black.
    expect(px(ctx, 0, 0)).toEqual([0, 0, 0, 255]);
    expect(px(ctx, 47, 31)).toEqual(framePx(55, 39));
  });

  it("base crop matches __golden__/base-crop.png", async () => {
    const { canvas } = draw([]);
    await expectGolden("base-crop", canvas);
  });

  it("clamps a selection that overhangs the frame", () => {
    const canvas = createCanvas(8, 8);
    const ctx = canvas.getContext("2d");
    render<SkiaImageData>(FRAME, { x: 60, y: 44, w: 8, h: 8 }, new AnnotationDocument(), ctx);
    expect(px(ctx, 3, 3)).toEqual(framePx(63, 47));
  });
});

describe("render(): one primitive per golden", () => {
  it("pen: 3-point polyline, size 3, red", async () => {
    const { canvas, ctx } = draw([
      { kind: "pen", pts: [{ x: 4, y: 28 }, { x: 20, y: 6 }, { x: 44, y: 26 }], color: RED, size: 3 },
    ]);
    expect(px(ctx, 20, 6)[0]).toBeGreaterThan(200);
    await expectGolden("pen", canvas);
  });

  it("line: single segment", async () => {
    const { canvas, ctx } = draw([{ kind: "line", a: { x: 4, y: 4 }, b: { x: 44, y: 28 }, color: RED, size: 3 }]);
    expect(px(ctx, 24, 16)[0]).toBeGreaterThan(200);
    await expectGolden("line", canvas);
  });

  it("arrow: segment with a filled triangle head at b", async () => {
    const { canvas, ctx } = draw([{ kind: "arrow", a: { x: 4, y: 16 }, b: { x: 44, y: 16 }, color: RED, size: 3 }]);
    // Head is max(8, size*4) = 12 px long: a pixel 4 px off the axis near b is inside
    // the triangle, while the same offset mid-shaft is untouched frame.
    expect(px(ctx, 36, 12)[0]).toBeGreaterThan(200);
    expect(px(ctx, 36, 12)[1]).toBeLessThan(60);
    expect(px(ctx, 20, 12)).toEqual(framePx(28, 20));
    await expectGolden("arrow", canvas);
  });

  it("rect: outline only", async () => {
    const { canvas, ctx } = draw([{ kind: "rect", r: { x: 6, y: 6, w: 36, h: 20 }, color: RED, size: 3 }]);
    expect(px(ctx, 6, 16)[0]).toBeGreaterThan(200);
    expect(px(ctx, 24, 16)).toEqual(framePx(32, 24)); // interior untouched
    await expectGolden("rect", canvas);
  });

  it("ellipse: outline only", async () => {
    const { canvas, ctx } = draw([{ kind: "ellipse", r: { x: 6, y: 6, w: 36, h: 20 }, color: RED, size: 3 }]);
    expect(px(ctx, 6, 16)[0]).toBeGreaterThan(200);
    expect(px(ctx, 24, 16)).toEqual(framePx(32, 24));
    await expectGolden("ellipse", canvas);
  });

  it("marker: yellow at alpha 0.5, the black grid stays visible through it", async () => {
    const { canvas, ctx } = draw([
      { kind: "marker", pts: [{ x: 4, y: 16 }, { x: 44, y: 16 }], color: "#ffff00", size: 3 },
    ]);
    // Crop (16,16) is frame (24,24): on the grid (24%8=0), black under the stroke.
    const [r, g, b] = px(ctx, 16, 16);
    expect(r).toBeGreaterThan(90);
    expect(r).toBeLessThan(170);
    expect(g).toBeGreaterThan(90);
    expect(g).toBeLessThan(170);
    expect(b).toBeLessThan(40);
    await expectGolden("marker", canvas);
  });

  it("blur: block 8 pixelates a region into uniform cells sampled from the original crop", async () => {
    const { canvas, ctx } = draw([{ kind: "blur", r: { x: 8, y: 8, w: 32, h: 16 }, block: 8 }]);
    const cell = ctx.getImageData(8, 8, 8, 8).data;
    for (let i = 4; i < cell.length; i += 4) {
      expect([cell[i], cell[i + 1], cell[i + 2]]).toEqual([cell[0], cell[1], cell[2]]);
    }
    // The cell average is neither pure black nor the gradient at one pixel.
    expect(cell[0]).toBeGreaterThan(0);
    expect(px(ctx, 2, 2)).toEqual(framePx(10, 10)); // outside the blur rect untouched
    await expectGolden("blur", canvas);
  });

  it("text: renders via fillText (no golden in Phase 1 — RESEARCH A14, Text tool is Phase 3)", () => {
    const { ctx } = draw([{ kind: "text", at: { x: 4, y: 24 }, text: "Hi", color: RED, size: 4 }]);
    let red = 0;
    const d = ctx.getImageData(0, 0, SEL.w, SEL.h).data;
    for (let i = 0; i < d.length; i += 4) if (d[i]! > 200 && d[i + 1]! < 80 && d[i + 2]! < 80) red += 1;
    expect(red).toBeGreaterThan(0);
  });
});

describe("golden harness (T-01-11)", () => {
  const dir = mkdtempSync(join(tmpdir(), "snapshade-golden-test-"));
  afterAll(() => rmSync(dir, { recursive: true, force: true }));

  it("UPDATE_GOLDEN=1 writes the PNG instead of comparing", async () => {
    const { canvas } = draw([]);
    await expectGolden("harness-write", canvas, { goldenDir: dir, env: { UPDATE_GOLDEN: "1" } });
    expect(existsSync(join(dir, "harness-write.png"))).toBe(true);
    // And a second run without the flag compares against what was written.
    await expectGolden("harness-write", canvas, { goldenDir: dir, env: {} });
  });

  it("a missing golden FAILS naming the file and never auto-creates it", async () => {
    const { canvas } = draw([]);
    await expect(expectGolden("harness-missing", canvas, { goldenDir: dir, env: {} })).rejects.toThrow(
      /missing golden .*harness-missing\.png/,
    );
    expect(existsSync(join(dir, "harness-missing.png"))).toBe(false);
  });

  it("a mismatch fails and writes the diff under os.tmpdir(), not the repo", async () => {
    const { canvas } = draw([]);
    await expectGolden("harness-diff", canvas, { goldenDir: dir, env: { UPDATE_GOLDEN: "1" } });
    const other = draw([{ kind: "rect", r: { x: 0, y: 0, w: 48, h: 32 }, color: RED, size: 12 }]).canvas;
    const diffDir = join(dir, "diff-out");
    await expect(expectGolden("harness-diff", other, { goldenDir: dir, env: {}, diffDir })).rejects.toThrow(
      /pixels differ/,
    );
    expect(existsSync(join(diffDir, "harness-diff.png"))).toBe(true);
  });
});
