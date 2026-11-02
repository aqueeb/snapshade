//! An [`OverlayHost`] that only records what was asked of it.

use snapshade_core::topology::DisplayTopology;
use snapshade_platform_api::{OverlayHost, PlatformError};

use crate::FakeOverlayHost;

impl FakeOverlayHost {
    fn prepared(&self) -> bool {
        self.calls.contains(&"prepare")
    }

    fn record(&mut self, call: &'static str) -> Result<(), PlatformError> {
        if !self.prepared() {
            return Err(PlatformError::Os(format!(
                "{call} called before prepare: no overlay windows exist"
            )));
        }
        self.calls.push(call);
        Ok(())
    }
}

impl OverlayHost for FakeOverlayHost {
    fn prepare(&mut self, _topo: &DisplayTopology) -> Result<(), PlatformError> {
        self.calls.push("prepare");
        Ok(())
    }

    fn show_all(&mut self) -> Result<(), PlatformError> {
        self.record("show_all")
    }

    fn hide_all(&mut self) -> Result<(), PlatformError> {
        self.record("hide_all")
    }
}
