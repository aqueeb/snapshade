// Golden-image harness (D-04, RESEARCH Q8, threat T-01-11).
//
// - Compares a rendered @napi-rs/canvas against `src/core/render/__golden__/<name>.png`
//   with pixelmatch (threshold 0.1, includeAA false) and fails when more than 0.1 %
//   of the pixels differ.
// - A MISSING golden fails with a message naming the file; it is never created
//   implicitly. Only `UPDATE_GOLDEN=1` rewrites goldens.
// - On failure the diff PNG is written OUTSIDE the repo (os.tmpdir()) and its path
//   is printed, so a regression can never silently commit a new golden.
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createCanvas, loadImage, type Canvas } from "@napi-rs/canvas";
import pixelmatch from "pixelmatch";

export const GOLDEN_DIR = resolve(dirname(fileURLToPath(import.meta.url)), "../src/core/render/__golden__");
export const DIFF_DIR = join(tmpdir(), "snapshade-golden-diff");

export interface GoldenOptions {
  /** Directory holding `<name>.png`; defaults to the committed golden dir. */
  goldenDir?: string;
  /** Environment to consult for UPDATE_GOLDEN; defaults to `process.env`. */
  env?: Record<string, string | undefined>;
  /** Where a failing diff is written; defaults to `os.tmpdir()/snapshade-golden-diff`. */
  diffDir?: string;
}

export async function expectGolden(name: string, canvas: Canvas, opts: GoldenOptions = {}): Promise<void> {
  const goldenDir = opts.goldenDir ?? GOLDEN_DIR;
  const env = opts.env ?? process.env;
  const goldenPath = join(goldenDir, `${name}.png`);

  if (env.UPDATE_GOLDEN === "1") {
    mkdirSync(goldenDir, { recursive: true });
    writeFileSync(goldenPath, await canvas.encode("png"));
    return;
  }

  if (!existsSync(goldenPath)) {
    throw new Error(
      `missing golden ${goldenPath} — goldens are never auto-created; run UPDATE_GOLDEN=1 bun run --cwd ui test and review the PNG before committing`,
    );
  }

  const expectedImg = await loadImage(goldenPath);
  const w = canvas.width;
  const h = canvas.height;
  if (expectedImg.width !== w || expectedImg.height !== h) {
    throw new Error(`golden ${goldenPath} is ${expectedImg.width}x${expectedImg.height}, render is ${w}x${h}`);
  }
  const expectedCanvas = createCanvas(w, h);
  const ectx = expectedCanvas.getContext("2d");
  ectx.drawImage(expectedImg, 0, 0);
  const expected = ectx.getImageData(0, 0, w, h);
  const actual = canvas.getContext("2d").getImageData(0, 0, w, h);
  const diffCanvas = createCanvas(w, h);
  const dctx = diffCanvas.getContext("2d");
  const diff = dctx.createImageData(w, h);

  const mismatch = pixelmatch(expected.data, actual.data, diff.data, w, h, { threshold: 0.1, includeAA: false });
  const ratio = mismatch / (w * h);
  if (ratio > 0.001) {
    const diffDir = opts.diffDir ?? DIFF_DIR;
    mkdirSync(diffDir, { recursive: true });
    dctx.putImageData(diff, 0, 0);
    const diffPath = join(diffDir, `${name}.png`);
    writeFileSync(diffPath, await diffCanvas.encode("png"));
    const actualPath = join(diffDir, `${name}.actual.png`);
    writeFileSync(actualPath, await canvas.encode("png"));
    console.error(`golden mismatch for ${name}: ${mismatch} px (${(ratio * 100).toFixed(3)} %); diff ${diffPath}, actual ${actualPath}`);
    throw new Error(`golden ${name}: ${mismatch}/${w * h} pixels differ (> 0.1 %); see ${diffPath}`);
  }
}
