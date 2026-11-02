// Boundary tests for the spike gate statistics (Plan 01-07 Task 1, hardened in Plan 01-11; CAPTURE-02 boundary
// and ordering). Runs through Vitest via `bun run --cwd ui test` — never `bun test` (D-20).
import { describe, expect, it } from "vitest";
import { computeStats, parseArgs, summarize, type LatencyRowLike } from "../../scripts/spike-stats";

/** One well-formed row whose ordering invariant holds: t1 <= t_show <= max(t4) = total. */
function row(run: number, total_ms: number, overrides: Partial<LatencyRowLike> = {}): LatencyRowLike {
  return {
    schema: 1,
    run,
    scenario: "a-desktop",
    selftest: false,
    t1_ms: 20,
    t2_ms: { "1": 30 },
    t_show_ms: 40,
    t4_ms: { "1": total_ms },
    total_ms,
    placement_ok: true,
    ...overrides,
  };
}

/** 20 rows whose total_ms p95 (nearest rank: sorted[18]) is exactly `p95`. */
function rowsWithP95(p95: number): LatencyRowLike[] {
  const rows: LatencyRowLike[] = [];
  for (let i = 1; i <= 18; i++) rows.push(row(i, 100));
  rows.push(row(19, p95));
  rows.push(row(20, p95 + 50));
  return rows;
}

describe("computeStats", () => {
  it("returns n, median, nearest-rank p95, min, max for 20 values", () => {
    const values = Array.from({ length: 20 }, (_, i) => (i + 1) * 10); // 10..200
    expect(computeStats(values)).toEqual({ n: 20, median: 105, p95: 190, min: 10, max: 200 });
  });
});

describe("summarize gate boundary", () => {
  it("passes when p95 total_ms is 149.9 with a 150 ms gate", () => {
    const { violations } = summarize(rowsWithP95(149.9), { requireN: 20, gateMs: 150 });
    expect(violations).toEqual([]);
  });

  it("fails when p95 total_ms is exactly 150.0", () => {
    const { violations } = summarize(rowsWithP95(150.0), { requireN: 20, gateMs: 150 });
    expect(violations.some((v) => /p95 total_ms/.test(v) && />= 150/.test(v))).toBe(true);
  });

  it("fails when p95 total_ms is 150.1", () => {
    const { violations } = summarize(rowsWithP95(150.1), { requireN: 20, gateMs: 150 });
    expect(violations.some((v) => /p95 total_ms/.test(v) && />= 150/.test(v))).toBe(true);
  });

  it("fails when a scenario has fewer rows than --require n", () => {
    const rows = rowsWithP95(100).slice(0, 19);
    const { violations } = summarize(rows, { requireN: 20, gateMs: 150 });
    expect(violations.some((v) => /n=19 < 20/.test(v))).toBe(true);
  });
});

describe("summarize ordering and placement invariants", () => {
  it("flags a row whose t1_ms exceeds t_show_ms, naming the run", () => {
    const rows = [row(1, 80), row(7, 80, { t1_ms: 50, t_show_ms: 45 })];
    const { violations } = summarize(rows, { gateMs: 150 });
    expect(violations.some((v) => /run 7/.test(v) && /t1_ms/.test(v))).toBe(true);
  });

  it("flags a row whose t_show_ms exceeds max(t4_ms), naming the run", () => {
    const rows = [row(1, 80), row(9, 80, { t_show_ms: 90, t4_ms: { "1": 80, "2": 85 } })];
    const { violations } = summarize(rows, { gateMs: 150 });
    expect(violations.some((v) => /run 9/.test(v) && /t_show_ms/.test(v))).toBe(true);
  });

  it("flags a row with placement_ok false, naming the run", () => {
    const rows = [row(1, 80), row(3, 80, { placement_ok: false })];
    const { violations } = summarize(rows, { gateMs: 150 });
    expect(violations.some((v) => /run 3/.test(v) && /placement_ok/.test(v))).toBe(true);
  });
});

describe("summarize scenario selection", () => {
  const rows = [
    row(1, 80),
    row(2, 81),
    row(3, 82, { scenario: "selftest", selftest: true }),
    row(4, 83, { scenario: "selftest", selftest: true }),
    row(5, 84, { scenario: "b-fullscreen-safari" }),
    row(6, 85, { scenario: "b-fullscreen-safari" }),
  ];

  it("excludes selftest rows from the tables by default", () => {
    const { tables } = summarize(rows, { gateMs: 150 });
    expect(tables).toContain("a-desktop");
    expect(tables).toContain("b-fullscreen-safari");
    expect(tables).not.toContain("selftest");
  });

  it("includes selftest rows when includeSelftest is set", () => {
    const { tables } = summarize(rows, { gateMs: 150, includeSelftest: true });
    expect(tables).toContain("selftest");
  });

  it("filters to one scenario tag", () => {
    const { tables } = summarize(rows, { gateMs: 150, scenario: "b-fullscreen-safari" });
    expect(tables).toContain("b-fullscreen-safari");
    expect(tables).not.toContain("a-desktop");
  });
});

describe("summarize target report", () => {
  it("reports the 100 ms target without affecting the violations", () => {
    const rows = Array.from({ length: 5 }, (_, i) => row(i + 1, 120));
    const { tables, violations } = summarize(rows, { gateMs: 150, targetMs: 100 });
    expect(tables).toMatch(/target 100 ms/);
    expect(tables).toMatch(/total_ms/);
    expect(violations).toEqual([]);
  });

  it("states that the median meets the target when it does", () => {
    const rows = Array.from({ length: 5 }, (_, i) => row(i + 1, 90));
    const { tables } = summarize(rows, { gateMs: 150, targetMs: 100 });
    expect(tables).toMatch(/target 100 ms: (met|MET)/);
  });
});

// ---------------------------------------------------------------------------------------
// Plan 01-11: instrument hardening (01-SPIKE-RESULT open issues 7 and 11; REVIEW WR-08;
// CAPTURE-02 boundary + precision probes).
// ---------------------------------------------------------------------------------------

describe("summarize zero-row gate (open issue 7)", () => {
  it("fails when --scenario selects no rows from an empty log and --require is set", () => {
    const { tables, violations } = summarize([], { scenario: "c-idle-battery", requireN: 20, gateMs: 150 });
    expect(violations).toContain("c-idle-battery: n=0 < 20 required runs");
    expect(tables).toContain("(no rows selected)");
  });

  it("fails when the tag matches nothing in a non-empty log", () => {
    const { violations } = summarize(rowsWithP95(100), { scenario: "c-idle-battery", requireN: 20, gateMs: 150 });
    expect(violations).toEqual(["c-idle-battery: n=0 < 20 required runs"]);
  });

  it("fails when --displays alone selects nothing and --require is set", () => {
    const rows = [row(1, 80, { displays: [{ id: 1 }] }), row(2, 81, { displays: [{ id: 1 }] })];
    const { tables, violations } = summarize(rows, { displays: 3, requireN: 20 });
    expect(tables).toContain("(no rows selected)");
    expect(violations).toEqual(["displays=3: n=0 < 20 required runs"]);
  });

  it("does not treat an empty log with no tag asked for as a gate run", () => {
    const { tables, violations } = summarize([], { requireN: 20 });
    expect(violations).toEqual([]);
    expect(tables).toContain("(no rows selected)");
  });
});

describe("summarize printed == gated p95 (precision)", () => {
  it("fails when p95 is 149.96: the table prints 150.0 and 150.0 is not < 150", () => {
    const { tables, violations } = summarize(rowsWithP95(149.96), { requireN: 20, gateMs: 150 });
    expect(tables).toContain("gate 150 ms: FAIL (p95 total_ms 150.0)");
    expect(violations).toContain("a-desktop: p95 total_ms 150.0 >= 150 ms gate");
  });

  it("passes when p95 is 149.94: the table prints 149.9", () => {
    const { tables, violations } = summarize(rowsWithP95(149.94), { requireN: 20, gateMs: 150 });
    expect(tables).toContain("gate 150 ms: PASS (p95 total_ms 149.9)");
    expect(violations).toEqual([]);
  });

  it("renders the median the same way on the (never gated) target line", () => {
    const rows = Array.from({ length: 5 }, (_, i) => row(i + 1, 99.96));
    const { tables, violations } = summarize(rows, { gateMs: 150, targetMs: 100 });
    expect(tables).toContain("target 100 ms: MISSED (median total_ms 100.0)");
    expect(violations).toEqual([]);
  });
});

describe("summarize --displays filter (open issue 11)", () => {
  const two = [{ id: 1 }, { id: 2 }];
  const three = [{ id: 1 }, { id: 2 }, { id: 3 }];
  const rows = [
    row(1, 80, { displays: two }),
    row(2, 81, { displays: two }),
    row(3, 200, { displays: three }),
    row(4, 201, { displays: three }),
    row(5, 300), // legacy row without displays[]
  ];

  it("keeps only rows whose displays[] has exactly n entries", () => {
    const { tables, violations } = summarize(rows, { displays: 2, gateMs: 150 });
    expect(tables).toContain("### a-desktop (n=2");
    expect(tables).toMatch(/\| total_ms \| 2 \| 80\.5 \| 81\.0 \| 80\.0 \| 81\.0 \|/);
    expect(violations).toEqual([]);
  });

  it("selects the 3-display rows of a mixed log with --displays 3", () => {
    const { tables, violations } = summarize(rows, { displays: 3, gateMs: 150, requireN: 2 });
    expect(tables).toContain("### a-desktop (n=2");
    expect(tables).toMatch(/\| total_ms \| 2 \| 200\.5 \| 201\.0 \| 200\.0 \| 201\.0 \|/);
    expect(violations).toEqual(["a-desktop: p95 total_ms 201.0 >= 150 ms gate"]);
  });

  it("drops rows without displays[] and notes how many were ignored", () => {
    const { tables } = summarize(rows, { displays: 2, gateMs: 150 });
    expect(tables).toContain("(1 row without displays[] ignored)");
    expect(tables).not.toContain("300.0");
  });

  it("does not mention displays[] when no --displays filter is asked for", () => {
    const { tables } = summarize(rows, { gateMs: 150 });
    expect(tables).toContain("### a-desktop (n=5");
    expect(tables).not.toContain("without displays[]");
  });
});

describe("summarize null t_show_ms (REVIEW WR-08)", () => {
  const rows = [row(1, 80), row(2, 82), row(5, 84, { t_show_ms: null })];

  it("flags the row as a violation naming the run", () => {
    const { violations } = summarize(rows, { gateMs: 150 });
    expect(violations).toContain("a-desktop run 5: t_show_ms missing");
  });

  it("excludes the row from the t_show_ms statistics but not from t1_ms / total_ms", () => {
    const { tables } = summarize(rows, { gateMs: 150 });
    expect(tables).toMatch(/\| t_show_ms \| 2 \| 40\.0 \| 40\.0 \| 40\.0 \| 40\.0 \|/);
    expect(tables).toMatch(/\| t1_ms \| 3 \|/);
    expect(tables).toMatch(/\| total_ms \| 3 \| 82\.0 \| 84\.0 \| 80\.0 \| 84\.0 \|/);
  });

  it("skips the ordering checks for that row instead of comparing against null", () => {
    const { violations } = summarize(rows, { gateMs: 150 });
    expect(violations.filter((v) => /run 5/.test(v))).toEqual(["a-desktop run 5: t_show_ms missing"]);
  });
});

describe("parseArgs --displays", () => {
  it("parses --displays 2 into opts.displays", () => {
    expect(parseArgs(["x.jsonl", "--displays", "2"]).opts.displays).toBe(2);
  });

  it("throws without a numeric value, like the other numeric flags", () => {
    expect(() => parseArgs(["x.jsonl", "--displays"])).toThrow(/--displays needs a numeric value/);
    expect(() => parseArgs(["x.jsonl", "--displays", "two"])).toThrow(/--displays needs a numeric value/);
  });
});
