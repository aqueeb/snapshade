//! Scripted display topologies, fed from the shared geometry fixture (D-05).

use std::collections::BTreeMap;

use serde::Deserialize;
use snapshade_core::geometry::{DisplayId, DisplayInfo, LogicalRect};
use snapshade_core::topology::DisplayTopology;
use snapshade_platform_api::{DisplayProvider, PlatformError};

/// The same file the Rust and TypeScript geometry tests read; only the
/// `layouts` map is used here.
const FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../fixtures/geometry/displays.json"
));

#[derive(Deserialize)]
struct Fixture {
    layouts: BTreeMap<String, Layout>,
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

use crate::FakeDisplayProvider;

impl FakeDisplayProvider {
    /// Report exactly `topology`.
    pub fn new(topology: DisplayTopology) -> Self {
        Self { topology }
    }

    /// Build the topology of the named layout in `fixtures/geometry/displays.json`
    /// (e.g. `"retina-plus-1x-right"`). Panics on an unknown layout name so a
    /// typo in a test cannot silently become an empty topology.
    pub fn from_layout(name: &str) -> Self {
        Self::new(layout_topology(name))
    }
}

impl DisplayProvider for FakeDisplayProvider {
    fn snapshot(&self) -> Result<DisplayTopology, PlatformError> {
        Ok(self.topology.clone())
    }
}

fn layout_topology(name: &str) -> DisplayTopology {
    let fixture: Fixture = serde_json::from_str(FIXTURE).expect("geometry fixture parses");
    let layout = fixture
        .layouts
        .get(name)
        .unwrap_or_else(|| panic!("unknown fixture layout '{name}'"));
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
