// Fixture-driven geometry test (D-05): executes every case of the shared fixture
// exactly like crates/core/tests/geometry_fixture.rs so the two implementations
// cannot drift. Physical outputs compare as exact integers, logical to 1e-9.
import { describe, expect, it } from "vitest";
import fixture from "../../../fixtures/geometry/displays.json";
import {
  flipNsScreenRect,
  hitTest,
  newTopology,
  physicalSizeFor,
  roundHalfUp,
  toLogical,
  toPhysical,
  type DisplayInfo,
  type DisplayTopology,
} from "./geometry";

interface FixtureDisplay {
  id: number;
  logical: [number, number, number, number];
  scale: number;
  primary: boolean;
}
interface FixtureCase {
  name: string;
  layout: string;
  op: string;
  display?: number;
  in?: [number, number];
  expect?: [number, number] | number | null;
  primary_height_pt?: number;
  in_nsrect?: [number, number, number, number];
  expect_logical?: [number, number, number, number];
}

const layouts = fixture.layouts as unknown as Record<string, { displays: FixtureDisplay[] }>;
const cases = fixture.cases as unknown as FixtureCase[];

function displayInfo(d: FixtureDisplay): DisplayInfo {
  const logical = { x: d.logical[0], y: d.logical[1], w: d.logical[2], h: d.logical[3] };
  return {
    id: d.id,
    logical,
    scale: d.scale,
    physical: physicalSizeFor(logical, d.scale),
    primary: d.primary,
  };
}

function topologyFor(layout: string): DisplayTopology {
  const l = layouts[layout];
  if (!l) throw new Error(`unknown layout ${layout}`);
  return newTopology(l.displays.map(displayInfo));
}

function displayOf(t: DisplayTopology, id: number): DisplayInfo {
  const d = t.displays.find((x) => x.id === id);
  if (!d) throw new Error(`display ${id} not in topology`);
  return d;
}

function pair(v: FixtureCase["in"] | FixtureCase["expect"]): [number, number] {
  if (!Array.isArray(v) || v.length !== 2) throw new Error("expected a 2-tuple");
  return [v[0], v[1]];
}

describe("fixtures/geometry/displays.json (shared with crates/core)", () => {
  it("has at least 14 cases", () => {
    expect(cases.length).toBeGreaterThanOrEqual(14);
  });

  let executed = 0;
  for (const c of cases) {
    it(`case ${c.name}`, () => {
      const topo = topologyFor(c.layout);
      switch (c.op) {
        case "physical_size": {
          const d = displayOf(topo, c.display!);
          const got = physicalSizeFor(d.logical, d.scale);
          const [w, h] = pair(c.expect);
          expect([got.w, got.h]).toEqual([w, h]);
          break;
        }
        case "to_physical": {
          const d = displayOf(topo, c.display!);
          const [x, y] = pair(c.in);
          const got = toPhysical(d, { x, y });
          const [ex, ey] = pair(c.expect);
          expect([got.x, got.y]).toEqual([ex, ey]);
          break;
        }
        case "to_logical": {
          const d = displayOf(topo, c.display!);
          const [x, y] = pair(c.in);
          const got = toLogical(d, { x, y });
          const [ex, ey] = pair(c.expect);
          expect(got.x).toBeCloseTo(ex, 9);
          expect(got.y).toBeCloseTo(ey, 9);
          break;
        }
        case "hit_test": {
          const [x, y] = pair(c.in);
          const got = hitTest(topo, { x, y });
          expect(got).toBe(c.expect === null ? null : (c.expect as number));
          break;
        }
        case "flip_nsscreen": {
          const r = c.in_nsrect!;
          const got = flipNsScreenRect(c.primary_height_pt!, r[0], r[1], r[2], r[3]);
          const e = c.expect_logical!;
          expect(got.x).toBeCloseTo(e[0], 9);
          expect(got.y).toBeCloseTo(e[1], 9);
          expect(got.w).toBeCloseTo(e[2], 9);
          expect(got.h).toBeCloseTo(e[3], 9);
          break;
        }
        default:
          throw new Error(`unknown op ${c.op}`);
      }
      executed += 1;
    });
  }

  it("executed every case (>= 14)", () => {
    expect(executed).toBe(cases.length);
    expect(executed).toBeGreaterThanOrEqual(14);
  });
});

describe("geometry unit rules", () => {
  it("roundHalfUp is floor(v + 0.5): half toward +inf, unlike Math.round at -0.5 in Rust", () => {
    expect(roundHalfUp(-0.5)).toBe(0);
    expect(roundHalfUp(2.5)).toBe(3);
    expect(roundHalfUp(-1.5)).toBe(-1);
    expect(roundHalfUp(0.5)).toBe(1);
    expect(roundHalfUp(3.49)).toBe(3);
    expect(roundHalfUp(-3.51)).toBe(-4);
  });

  it("newTopology orders primary first, then ascending id", () => {
    const mk = (id: number, primary: boolean): DisplayInfo => ({
      id,
      logical: { x: 0, y: 0, w: 100, h: 100 },
      scale: 1,
      physical: { w: 100, h: 100 },
      primary,
    });
    expect(newTopology([mk(2, false), mk(1, true)]).displays[0]!.id).toBe(1);
    expect(newTopology([mk(3, false), mk(2, false), mk(1, true)]).displays.map((d) => d.id)).toEqual([
      1, 2, 3,
    ]);
    expect(newTopology([mk(1, false), mk(7, true)]).displays.map((d) => d.id)).toEqual([7, 1]);
  });
});
