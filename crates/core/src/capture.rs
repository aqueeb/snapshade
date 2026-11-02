//! Capture frame types shared by every platform adapter.

use std::fmt;

use crate::geometry::{DisplayId, PhysicalSize};

/// One display's frozen frame: tightly packed RGBA8, row-major, top-left origin,
/// `width * height * 4` bytes, at the display's native (physical) resolution.
#[derive(Clone, PartialEq)]
pub struct CaptureFrame {
    pub display_id: DisplayId,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    pub rgba: Vec<u8>,
}

impl CaptureFrame {
    /// The byte length a well-formed `rgba` buffer must have: `width * height * 4`.
    pub fn expected_len(&self) -> usize {
        self.width as usize * self.height as usize * 4
    }
}

impl fmt::Debug for CaptureFrame {
    // A 5K Retina frame is ~59 MB; never dump the pixel buffer into a log line.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CaptureFrame")
            .field("display_id", &self.display_id)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("scale", &self.scale)
            .field("rgba_len", &self.rgba.len())
            .finish()
    }
}

/// Every display's frame from one capture.
///
/// Invariant: `frames` is sorted by `display_id` ascending, whatever order the
/// per-display capture threads finished in (CAPTURE-01 ordering).
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureSet {
    pub frames: Vec<CaptureFrame>,
}

impl CaptureSet {
    /// Build a set, enforcing the ascending-`display_id` invariant.
    pub fn new(mut frames: Vec<CaptureFrame>) -> Self {
        frames.sort_by_key(|f| f.display_id);
        Self { frames }
    }

    /// The frame for `id`, if captured.
    pub fn get(&self, id: DisplayId) -> Option<&CaptureFrame> {
        self.frames.iter().find(|f| f.display_id == id)
    }
}

/// Why a capture was requested; drives whether the overlay is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureKind {
    /// Hotkey → overlay → drag-select (the main loop).
    Interactive,
    /// Instant-save-fullscreen hotkey: capture and save, no overlay.
    InstantFullscreen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureRequest {
    pub kind: CaptureKind,
    pub include_cursor: bool,
}

impl CaptureRequest {
    /// The main-loop request: interactive, cursor excluded (the "include the mouse
    /// cursor" option is off by default).
    pub fn interactive() -> Self {
        Self {
            kind: CaptureKind::Interactive,
            include_cursor: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CaptureError {
    #[error("no displays are connected")]
    NoDisplays,
    #[error("screen recording permission denied")]
    PermissionDenied,
    #[error("capture of display {display_id} failed: {reason}")]
    DisplayFailed {
        display_id: DisplayId,
        reason: String,
    },
    #[error("display {display_id} bitmap is {actual:?}, expected {expected:?}")]
    SizeMismatch {
        display_id: DisplayId,
        expected: PhysicalSize,
        actual: PhysicalSize,
    },
    #[error("capture is not supported on this platform")]
    Unsupported,
}

#[cfg(test)]
mod tests {
    use super::{CaptureFrame, CaptureKind, CaptureRequest, CaptureSet};

    fn frame(id: u32, w: u32, h: u32) -> CaptureFrame {
        CaptureFrame {
            display_id: id,
            width: w,
            height: h,
            scale: 1.0,
            rgba: vec![0; (w * h * 4) as usize],
        }
    }

    #[test]
    fn new_sorts_frames_by_display_id_ascending() {
        let set = CaptureSet::new(vec![frame(3, 1, 1), frame(1, 1, 1), frame(2, 1, 1)]);
        let ids: Vec<u32> = set.frames.iter().map(|f| f.display_id).collect();
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[test]
    fn get_returns_the_frame_for_an_id() {
        let set = CaptureSet::new(vec![frame(3, 1, 1), frame(1, 1, 1), frame(2, 5, 7)]);
        let f = set.get(2).expect("display 2 captured");
        assert_eq!((f.width, f.height), (5, 7));
        assert!(set.get(9).is_none());
    }

    #[test]
    fn expected_len_is_width_times_height_times_four() {
        assert_eq!(frame(1, 5, 7).expected_len(), 5 * 7 * 4);
        assert_eq!(frame(1, 3024, 1964).expected_len(), 3024 * 1964 * 4);
    }

    #[test]
    fn interactive_request_excludes_the_cursor() {
        let req = CaptureRequest::interactive();
        assert_eq!(req.kind, CaptureKind::Interactive);
        assert!(!req.include_cursor);
    }
}
