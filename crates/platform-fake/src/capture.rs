//! Synthetic frames: one solid-colour `CaptureFrame` per display.

use snapshade_core::capture::{CaptureError, CaptureFrame, CaptureRequest, CaptureSet};
use snapshade_core::topology::DisplayTopology;
use snapshade_platform_api::{Permission, ScreenCapturer};

use crate::FakeCapturer;

impl ScreenCapturer for FakeCapturer {
    fn permission(&self) -> Permission {
        Permission::Granted
    }

    fn request_permission(&self) {}

    fn capture_all(
        &self,
        topo: &DisplayTopology,
        _req: &CaptureRequest,
    ) -> Result<CaptureSet, CaptureError> {
        if topo.is_empty() {
            return Err(CaptureError::NoDisplays);
        }
        // Walk the topology in reverse to mimic per-display capture threads
        // finishing out of order; CaptureSet::new restores ascending id order.
        let mut frames = Vec::with_capacity(topo.displays.len());
        for display in topo.displays.iter().rev() {
            if self.fail_display == Some(display.id) {
                return Err(CaptureError::DisplayFailed {
                    display_id: display.id,
                    reason: "scripted failure".to_string(),
                });
            }
            let pixels = display.physical.w as usize * display.physical.h as usize;
            let rgba = self.fill.repeat(pixels);
            frames.push(CaptureFrame {
                display_id: display.id,
                width: display.physical.w,
                height: display.physical.h,
                scale: display.scale,
                rgba,
            });
        }
        Ok(CaptureSet::new(frames))
    }
}
