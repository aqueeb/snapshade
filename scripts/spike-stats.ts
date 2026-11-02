// Spike latency statistics (RESEARCH Q6 line 471, Q12 "Spike stats"; D-10, D-12; hardened
// in Plan 01-11 for 01-SPIKE-RESULT open issues 7 and 11 and REVIEW WR-08).
//
// Reads `~/Library/Logs/Snapshade/spike-latency.jsonl` (schema 1, written by
// `src-tauri/src/latency.rs`), groups rows by `scenario`, and prints one Markdown table per
// scenario (metric | n | median | p95 | min | max) for t1_ms, t2_ms, t_show_ms, t4_ms and
// total_ms, where the per-display maps t2_ms / t4_ms are reduced to their max per row.
// p95 is nearest-rank: sorted[ceil(0.95 * n) - 1] (the 19th of 20).
//
// Gate (CAPTURE-02): exit 1 when any scenario has n < --require, p95 total_ms >= --gate-ms,
// a row that violates capture-then-show ordering (t1_ms <= t_show_ms <= max t4_ms), a row
// with placement_ok false, a row whose t_show_ms is null (the Rust `Option<f64>` is `None`
// when `record_shown` never ran — WR-08; such a row is excluded from the t_show_ms statistics
// instead of coercing to 0), or — when a selection was asked for with --scenario or
// --displays together with --require — a selection of ZERO rows (`<tag>: n=0 < N required
// runs`): a wrong tag or a wrong display count can no longer produce a vacuous exit 0 (open
// issue 7, T-01-31). An empty log with no selection asked for still prints `(no rows
// selected)` and exits 0 — that is not a gate run. The 100 ms target is REPORTED, never gated.
// Self-test rows (`selftest: true`) are excluded unless --include-selftest (D-10 honesty, T-01-19).
//
// Printed == gated (CAPTURE-02 precision contract): the gate and the target compare exactly the
// 0.1 ms figure the table prints — `gatedValue(v) = Number(v.toFixed(1))` — so a p95 of 149.96
// prints `150.0` and FAILS a 150 ms gate while 149.94 prints `149.9` and PASSES; 149.9 passes
// and 150.0 fails as before (strict `<`). Tie rule: `toFixed(1)` rounds the IEEE-754 double as
// stored, so 149.95 (stored as 149.9499…) prints `149.9` and passes — there is no separate
// rounding step that could disagree with the table.
//
// --displays <n> keeps only rows whose `displays[]` has exactly n entries so one log can be
// gated per topology (Retina + one 1x = 2, the 3-display DisplayLink dock = 3; open issue 11);
// rows without a `displays[]` field are dropped and counted in a `(k rows without displays[]
// ignored)` note.
//
// Runs under Bun as a CLI (`bun scripts/spike-stats.ts <jsonl> [flags]`; exit 2 on an
// unreadable file) and is imported by Vitest (`ui/test/spike-stats.test.ts`) for its unit
// tests — the CLI entry is guarded by `import.meta.main`. Never run it with `bun test`.

import { readFileSync } from "node:fs";

/** The fields of one `spike-latency.jsonl` row the statistics need (schema 1). */
export interface LatencyRowLike {
  schema: number;
  run: number;
  scenario: string;
  selftest: boolean;
  t1_ms: number;
  t2_ms: Record<string, number>;
  /** `null` when the row writer never saw `record_shown` (Rust `Option<f64>`, WR-08). */
  t_show_ms: number | null;
  t4_ms: Record<string, number>;
  total_ms: number;
  placement_ok: boolean;
  fmt?: string;
  /** Display geometry snapshot (RESEARCH Q6 `displays[]`); its length drives `--displays`. */
  displays?: unknown[];
}

export interface Stats {
  n: number;
  median: number;
  p95: number;
  min: number;
  max: number;
}

export interface SummarizeOptions {
  /** Minimum rows per scenario; a scenario below it is a violation. */
  requireN?: number;
  /** Hard gate: a scenario whose p95 total_ms is >= this many ms is a violation. */
  gateMs?: number;
  /** Reported target (default 100 ms): never affects the violations. */
  targetMs?: number;
  /** Count `selftest: true` rows (default false). */
  includeSelftest?: boolean;
  /** Only this scenario tag. */
  scenario?: string;
  /** Only rows whose `displays[]` has exactly this many entries. */
  displays?: number;
}

export interface Summary {
  tables: string;
  violations: string[];
}

const METRICS = ["t1_ms", "t2_ms", "t_show_ms", "t4_ms", "total_ms"] as const;
type Metric = (typeof METRICS)[number];
const DEFAULT_TARGET_MS = 100;

function at(sorted: number[], index: number): number {
  const v = sorted[index];
  return v === undefined ? Number.NaN : v;
}

/** n, median, nearest-rank p95 (`sorted[ceil(0.95 * n) - 1]`), min, max. */
export function computeStats(values: number[]): Stats {
  const n = values.length;
  if (n === 0) {
    return { n: 0, median: Number.NaN, p95: Number.NaN, min: Number.NaN, max: Number.NaN };
  }
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(n / 2);
  const median = n % 2 === 1 ? at(sorted, mid) : (at(sorted, mid - 1) + at(sorted, mid)) / 2;
  const p95 = at(sorted, Math.ceil(0.95 * n) - 1);
  return { n, median, p95, min: at(sorted, 0), max: at(sorted, n - 1) };
}

/**
 * The 0.1 ms figure the table prints, as a number (`Number(v.toFixed(1))`). Every gate and
 * target comparison runs on this value, so the verdict is about exactly the printed figure.
 */
export function gatedValue(v: number): number {
  return Number(v.toFixed(1));
}

/** Max over the displays of a per-display map; -Infinity when empty. */
function maxOverDisplays(map: Record<string, number>): number {
  return Object.values(map).reduce((acc, v) => Math.max(acc, v), Number.NEGATIVE_INFINITY);
}

/** `t_show_ms` as a finite number, or `undefined` when the row has none (null/absent/NaN). */
function tShowOf(row: LatencyRowLike): number | undefined {
  const v = row.t_show_ms;
  return typeof v === "number" && Number.isFinite(v) ? v : undefined;
}

/** Per-row metric values for the statistics; rows without a finite `t_show_ms` contribute none. */
function metricValues(group: LatencyRowLike[], metric: Metric): number[] {
  switch (metric) {
    case "t2_ms":
      return group.map((r) => maxOverDisplays(r.t2_ms));
    case "t4_ms":
      return group.map((r) => maxOverDisplays(r.t4_ms));
    case "t_show_ms":
      return group.map(tShowOf).filter((v): v is number => v !== undefined);
    default:
      return group.map((r) => r[metric]);
  }
}

function fmt1(v: number): string {
  return Number.isFinite(v) ? v.toFixed(1) : "-";
}

/** Ordering (capture → show → paint), missing-t_show and placement checks for one row. */
function rowViolations(row: LatencyRowLike): string[] {
  const out: string[] = [];
  const where = `${row.scenario} run ${row.run}`;
  const maxT4 = maxOverDisplays(row.t4_ms);
  const tShow = tShowOf(row);
  if (tShow === undefined) {
    // The missing stamp already fails the gate; comparing against it would be meaningless.
    out.push(`${where}: t_show_ms missing`);
  } else {
    if (row.t1_ms > tShow) {
      out.push(
        `${where}: t1_ms ${fmt1(row.t1_ms)} > t_show_ms ${fmt1(tShow)} (capture must finish before show)`,
      );
    }
    if (tShow > maxT4) {
      out.push(`${where}: t_show_ms ${fmt1(tShow)} > max t4_ms ${fmt1(maxT4)} (show must precede paint)`);
    }
  }
  if (!row.placement_ok) {
    out.push(`${where}: placement_ok false`);
  }
  return out;
}

/** Group rows, compute per-scenario tables, and collect gate violations. */
export function summarize(rows: LatencyRowLike[], opts: SummarizeOptions = {}): Summary {
  const targetMs = opts.targetMs ?? DEFAULT_TARGET_MS;
  let withoutDisplays = 0;
  const selected = rows
    .filter((r) => opts.includeSelftest || !r.selftest)
    .filter((r) => opts.scenario === undefined || r.scenario === opts.scenario)
    .filter((r) => {
      if (opts.displays === undefined) return true;
      if (!Array.isArray(r.displays)) {
        withoutDisplays++;
        return false;
      }
      return r.displays.length === opts.displays;
    });

  const byScenario = new Map<string, LatencyRowLike[]>();
  for (const row of selected) {
    const list = byScenario.get(row.scenario);
    if (list) list.push(row);
    else byScenario.set(row.scenario, [row]);
  }

  const violations: string[] = [];
  const lines: string[] = [];
  for (const scenario of [...byScenario.keys()].sort()) {
    const group = byScenario.get(scenario) ?? [];
    const fmts = [...new Set(group.map((r) => r.fmt).filter((f): f is string => f !== undefined))].sort();
    const fmtNote = fmts.length > 0 ? `, fmt=${fmts.join("/")}` : "";
    lines.push(`### ${scenario} (n=${group.length}${fmtNote})`);
    lines.push("");
    lines.push("| metric | n | median | p95 | min | max |");
    lines.push("|---|---|---|---|---|---|");
    let total: Stats | undefined;
    for (const metric of METRICS) {
      const stats = computeStats(metricValues(group, metric));
      if (metric === "total_ms") total = stats;
      lines.push(
        `| ${metric} | ${stats.n} | ${fmt1(stats.median)} | ${fmt1(stats.p95)} | ${fmt1(stats.min)} | ${fmt1(stats.max)} |`,
      );
    }
    lines.push("");
    if (total) {
      // Printed == gated: compare the 0.1 ms figures the table shows, never the raw doubles.
      const median = gatedValue(total.median);
      const met = median < targetMs;
      lines.push(`target ${targetMs} ms: ${met ? "met" : "MISSED"} (median total_ms ${fmt1(median)})`);
      if (opts.gateMs !== undefined) {
        const p95 = gatedValue(total.p95);
        const pass = p95 < opts.gateMs;
        lines.push(`gate ${opts.gateMs} ms: ${pass ? "PASS" : "FAIL"} (p95 total_ms ${fmt1(p95)})`);
        if (!pass) {
          violations.push(`${scenario}: p95 total_ms ${fmt1(p95)} >= ${opts.gateMs} ms gate`);
        }
      }
    }
    if (opts.requireN !== undefined && group.length < opts.requireN) {
      violations.push(`${scenario}: n=${group.length} < ${opts.requireN} required runs`);
    }
    for (const row of group) {
      violations.push(...rowViolations(row));
    }
    lines.push("");
  }

  if (byScenario.size === 0) {
    lines.push("(no rows selected)");
    // A selection that was asked for and came back empty is a failed gate run, not a pass.
    const asked = opts.scenario ?? (opts.displays !== undefined ? `displays=${opts.displays}` : undefined);
    if (asked !== undefined && opts.requireN !== undefined) {
      violations.push(`${asked}: n=0 < ${opts.requireN} required runs`);
    }
  }
  if (withoutDisplays > 0) {
    lines.push(`(${withoutDisplays} row${withoutDisplays === 1 ? "" : "s"} without displays[] ignored)`);
  }
  return { tables: lines.join("\n").trimEnd(), violations };
}

// ---------------------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------------------

interface CliArgs {
  path?: string;
  opts: SummarizeOptions;
}

function parseNumber(flag: string, raw: string | undefined): number {
  const value = Number(raw);
  if (raw === undefined || !Number.isFinite(value)) {
    throw new Error(`${flag} needs a numeric value, got ${raw ?? "nothing"}`);
  }
  return value;
}

export function parseArgs(argv: string[]): CliArgs {
  const out: CliArgs = { opts: {} };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === undefined) continue;
    switch (arg) {
      case "--require": {
        const raw = argv[++i];
        out.opts.requireN = parseNumber(arg, raw?.startsWith("n=") ? raw.slice(2) : raw);
        break;
      }
      case "--gate-ms":
        out.opts.gateMs = parseNumber(arg, argv[++i]);
        break;
      case "--target-ms":
        out.opts.targetMs = parseNumber(arg, argv[++i]);
        break;
      case "--scenario": {
        const raw = argv[++i];
        if (raw === undefined) throw new Error("--scenario needs a tag");
        out.opts.scenario = raw;
        break;
      }
      case "--displays": {
        const n = parseNumber(arg, argv[++i]);
        if (!Number.isInteger(n) || n < 1) throw new Error(`--displays needs a positive integer, got ${n}`);
        out.opts.displays = n;
        break;
      }
      case "--include-selftest":
        out.opts.includeSelftest = true;
        break;
      default:
        if (arg.startsWith("--")) throw new Error(`unknown flag ${arg}`);
        if (out.path !== undefined) throw new Error(`unexpected extra argument ${arg}`);
        out.path = arg;
    }
  }
  return out;
}

/** Parse JSON Lines; blank or corrupt lines are skipped with a warning. */
export function parseRows(text: string, warn: (msg: string) => void): LatencyRowLike[] {
  const rows: LatencyRowLike[] = [];
  const lines = text.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const line = (lines[i] ?? "").trim();
    if (line === "") continue;
    try {
      const parsed: unknown = JSON.parse(line);
      if (typeof parsed !== "object" || parsed === null || (parsed as { schema?: unknown }).schema !== 1) {
        warn(`skipping line ${i + 1}: not a schema 1 row`);
        continue;
      }
      rows.push(parsed as LatencyRowLike);
    } catch {
      warn(`skipping corrupt line ${i + 1}`);
    }
  }
  return rows;
}

function main(argv: string[]): number {
  let args: CliArgs;
  try {
    args = parseArgs(argv);
  } catch (err) {
    console.error(`spike-stats: ${err instanceof Error ? err.message : String(err)}`);
    console.error(
      "usage: bun scripts/spike-stats.ts <spike-latency.jsonl> [--require n=20] [--gate-ms 150] [--target-ms 100] [--scenario <tag>] [--displays <n>] [--include-selftest]",
    );
    return 2;
  }
  if (args.path === undefined) {
    console.error("spike-stats: missing the JSONL path");
    return 2;
  }
  let text: string;
  try {
    text = readFileSync(args.path, "utf8");
  } catch (err) {
    console.error(`spike-stats: cannot read ${args.path}: ${err instanceof Error ? err.message : String(err)}`);
    return 2;
  }
  const rows = parseRows(text, (msg) => console.error(`spike-stats: ${msg}`));
  const { tables, violations } = summarize(rows, args.opts);
  console.log(tables);
  console.log("");
  const excluded = args.opts.includeSelftest ? 0 : rows.filter((r) => r.selftest).length;
  if (excluded > 0) {
    console.log(`(${excluded} self-test row${excluded === 1 ? "" : "s"} excluded; pass --include-selftest to count them)`);
    console.log("");
  }
  if (violations.length > 0) {
    console.log(`Violations (${violations.length}):`);
    for (const v of violations) console.log(`- ${v}`);
    return 1;
  }
  console.log("Gate: PASS (no violations)");
  return 0;
}

if ((import.meta as ImportMeta & { main?: boolean }).main === true) {
  process.exit(main(process.argv.slice(2)));
}
