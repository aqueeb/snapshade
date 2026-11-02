//! Snapshade fake platform.
//!
//! Headless, in-memory implementation of the `snapshade-platform-api` traits
//! for unit and integration tests on every OS: scripted display topologies
//! (fed from `fixtures/geometry/displays.json`) and synthetic frames.
//!
//! The public types live here; their behaviour is in `displays`, `capture`,
//! and `overlay`.

use snapshade_core::geometry::DisplayId;
use snapshade_core::topology::DisplayTopology;

mod capture;
mod displays;
mod overlay;

/// A `DisplayProvider` that always reports one fixed topology; see
/// [`FakeDisplayProvider::from_layout`] for the fixture-driven constructor.
#[derive(Debug, Clone, PartialEq)]
pub struct FakeDisplayProvider {
    pub topology: DisplayTopology,
}

/// A `ScreenCapturer` that fills every display with `fill` and can be told to
/// fail on one display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeCapturer {
    /// RGBA value written to every pixel of every frame.
    pub fill: [u8; 4],
    /// When set, capturing that display returns `CaptureError::DisplayFailed`
    /// and no partial `Ok` is produced.
    pub fail_display: Option<DisplayId>,
}

impl Default for FakeCapturer {
    fn default() -> Self {
        Self {
            fill: [0, 0, 0, 255],
            fail_display: None,
        }
    }
}

/// An `OverlayHost` that records `"prepare"`, `"show_all"`, and `"hide_all"`
/// in call order. Showing or hiding before `prepare` is an error, as it would
/// be for real windows that do not exist yet.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FakeOverlayHost {
    pub calls: Vec<&'static str>,
}
