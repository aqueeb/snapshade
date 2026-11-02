// Belt-and-braces purity check (D-06, D-19): runs under `bun run --cwd ui test`
// even when ESLint is not invoked. ESLint's `no-restricted-imports` rule is the
// primary guard; this test greps every .ts file under src/core for the forbidden
// package scope.
import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const coreDir = dirname(fileURLToPath(import.meta.url));

// Assembled at runtime so this file does not itself contain the literal.
const forbidden = ["@tauri-", "apps"].join("");

function collectTsFiles(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) {
      collectTsFiles(full, out);
    } else if (entry.endsWith(".ts")) {
      out.push(full);
    }
  }
  return out;
}

describe("ui/src/core purity", () => {
  const files = collectTsFiles(coreDir);

  it("scans at least one file (an empty core must not pass vacuously)", () => {
    console.info(`purity: scanned ${files.length} files under src/core`);
    expect(files.length).toBeGreaterThan(0);
  });

  it(`no file under src/core contains ${forbidden}`, () => {
    const offenders = files
      .filter((file) => readFileSync(file, "utf8").includes(forbidden))
      .map((file) => relative(coreDir, file));
    expect(offenders).toEqual([]);
  });
});
