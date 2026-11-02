//! Fixture-driven geometry and topology tests (D-05).
//!
//! `fixtures/geometry/displays.json` is the single source of truth for the
//! expected values; the TypeScript suite (Plan 01-05) reads the same file. This
//! test must never edit the fixture, must panic on an `op` it does not know
//! (so a fixture addition cannot be silently skipped), and must execute at
//! least 14 cases. Physical outputs compare with exact integer equality,
//! logical outputs within 1e-9 (RESEARCH Q9).

use std::collections::BTreeMap;

use serde::Deserialize;
use snapshade_core::geometry::{
    flip_nsscreen_rect, DisplayId, DisplayInfo, LogicalPoint, LogicalRect, PhysicalPoint,
};
use snapshade_core::topology::DisplayTopology;

const FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../fixtures/geometry/displays.json"
));

const MIN_CASES: usize = 14;
const LOGICAL_EPS: f64 = 1e-9;

#[derive(Deserialize)]
struct Fixture {
    version: u32,
    layouts: BTreeMap<String, Layout>,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Layout {
    displays: Vec<FixtureDisplay>,
}

#[derive(Deserialize)]
struct FixtureDisplay {
    id: DisplayId,
    logical: [f64; 4],
    scale: f64,
    primary: bool,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    layout: Option<String>,
    op: String,
    display: Option<DisplayId>,
    #[serde(rename = "in")]
    input: Option<[f64; 2]>,
    expect: Option<serde_json::Value>,
    primary_height_pt: Option<f64>,
    in_nsrect: Option<[f64; 4]>,
    expect_logical: Option<[f64; 4]>,
}

fn topology_for(layout: &Layout) -> DisplayTopology {
    let displays = layout
        .displays
        .iter()
        .map(|d| {
            let logical = LogicalRect {
                x: d.logical[0],
                y: d.logical[1],
                w: d.logical[2],
                h: d.logical[3],
            };
            DisplayInfo {
                id: d.id,
                logical,
                scale: d.scale,
                physical: DisplayInfo::physical_size_for(&logical, d.scale),
                primary: d.primary,
            }
        })
        .collect();
    DisplayTopology::new(displays)
}

fn expect_ints(case: &Case) -> Vec<i64> {
    case.expect
        .as_ref()
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("case '{}': expect must be an array", case.name))
        .iter()
        .map(|v| {
            v.as_i64()
                .unwrap_or_else(|| panic!("case '{}': expect must hold integers", case.name))
        })
        .collect()
}

fn expect_floats(case: &Case) -> Vec<f64> {
    case.expect
        .as_ref()
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("case '{}': expect must be an array", case.name))
        .iter()
        .map(|v| {
            v.as_f64()
                .unwrap_or_else(|| panic!("case '{}': expect must hold numbers", case.name))
        })
        .collect()
}

fn approx_eq(a: f64, b: f64) -> bool {
    (a - b).abs() < LOGICAL_EPS
}

fn display<'a>(topo: &'a DisplayTopology, case: &Case) -> &'a DisplayInfo {
    let id = case
        .display
        .unwrap_or_else(|| panic!("case '{}': missing display", case.name));
    topo.get(id)
        .unwrap_or_else(|| panic!("case '{}': display {id} not in layout", case.name))
}

fn input_point(case: &Case) -> LogicalPoint {
    let [x, y] = case
        .input
        .unwrap_or_else(|| panic!("case '{}': missing in", case.name));
    LogicalPoint { x, y }
}

#[test]
fn every_fixture_case_passes() {
    let fixture: Fixture = serde_json::from_str(FIXTURE).expect("fixture parses");
    assert_eq!(fixture.version, 1, "fixture version");

    let mut ran = 0usize;
    for case in &fixture.cases {
        let name = case.name.as_str();
        let layout = case.layout.as_deref().map(|key| {
            fixture
                .layouts
                .get(key)
                .unwrap_or_else(|| panic!("case '{name}': unknown layout {key}"))
        });
        let topo = layout.map(topology_for);

        match case.op.as_str() {
            "physical_size" => {
                let topo = topo.as_ref().expect("physical_size needs a layout");
                let d = display(topo, case);
                let expect = expect_ints(case);
                assert_eq!(
                    [i64::from(d.physical.w), i64::from(d.physical.h)],
                    [expect[0], expect[1]],
                    "case '{name}': physical_size"
                );
            }
            "to_physical" => {
                let topo = topo.as_ref().expect("to_physical needs a layout");
                let d = display(topo, case);
                let got = d.to_physical(input_point(case));
                let expect = expect_ints(case);
                assert_eq!(
                    [i64::from(got.x), i64::from(got.y)],
                    [expect[0], expect[1]],
                    "case '{name}': to_physical"
                );
            }
            "to_logical" => {
                let topo = topo.as_ref().expect("to_logical needs a layout");
                let d = display(topo, case);
                let [x, y] = case.input.expect("to_logical needs in");
                let got = d.to_logical(PhysicalPoint {
                    x: x as i32,
                    y: y as i32,
                });
                let expect = expect_floats(case);
                assert!(
                    approx_eq(got.x, expect[0]) && approx_eq(got.y, expect[1]),
                    "case '{name}': to_logical got ({}, {}), expected ({}, {})",
                    got.x,
                    got.y,
                    expect[0],
                    expect[1]
                );
            }
            "hit_test" => {
                let topo = topo.as_ref().expect("hit_test needs a layout");
                let got = topo.hit_test(input_point(case));
                let expect =
                    match case.expect.as_ref() {
                        None | Some(serde_json::Value::Null) => None,
                        Some(v) => Some(v.as_u64().unwrap_or_else(|| {
                            panic!("case '{name}': expect must be an id or null")
                        }) as DisplayId),
                    };
                assert_eq!(got, expect, "case '{name}': hit_test");
            }
            "flip_nsscreen" => {
                let primary_h = case
                    .primary_height_pt
                    .unwrap_or_else(|| panic!("case '{name}': missing primary_height_pt"));
                let [x, y, w, h] = case
                    .in_nsrect
                    .unwrap_or_else(|| panic!("case '{name}': missing in_nsrect"));
                let got = flip_nsscreen_rect(primary_h, x, y, w, h);
                let e = case
                    .expect_logical
                    .unwrap_or_else(|| panic!("case '{name}': missing expect_logical"));
                assert!(
                    approx_eq(got.x, e[0])
                        && approx_eq(got.y, e[1])
                        && approx_eq(got.w, e[2])
                        && approx_eq(got.h, e[3]),
                    "case '{name}': flip_nsscreen got {got:?}, expected {e:?}"
                );
            }
            other => panic!("case '{name}': unknown fixture op '{other}'"),
        }
        ran += 1;
    }

    println!("geometry fixture: {ran} cases executed");
    assert!(
        ran >= MIN_CASES,
        "expected at least {MIN_CASES} fixture cases, ran {ran}"
    );
}
