//! Typed coordinate spaces (ARCHITECTURE Pattern 2).
//!
//! Three spaces exist and the type system keeps them apart:
//!
//! | Space                   | Type                              | Origin                                   |
//! |-------------------------|-----------------------------------|------------------------------------------|
//! | Global logical          | [`LogicalPoint`] / [`LogicalRect`] | OS virtual desktop, top-left, y-down     |
//! | Display-local physical  | [`PhysicalPoint`] / [`PhysicalRect`] | top-left of one display's bitmap       |
//!
//! Global logical space is **top-left origin, y-down** (CoreGraphics / Windows
//! convention). The macOS adapter flips `NSScreen.frame` once at the boundary
//! with [`flip_nsscreen_rect`]. Physical pixels are the source of truth for
//! selection, annotation, and export.
//!
//! Every conversion is pinned by `fixtures/geometry/displays.json`, which the
//! TypeScript core reads too (D-05).

use serde::{Deserialize, Serialize};

/// Stable per-display identifier (CGDirectDisplayID on macOS, an adapter-assigned
/// index on Windows). Primary display first, then ascending id, in a topology.
pub type DisplayId = u32;

/// A point in global logical space (points on macOS, DIPs on Windows).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LogicalPoint {
    pub x: f64,
    pub y: f64,
}

/// A rectangle in global logical space; `x`/`y` is the top-left corner.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LogicalRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// A point in display-local physical pixels (top-left of that display's bitmap).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysicalPoint {
    pub x: i32,
    pub y: i32,
}

/// A size in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysicalSize {
    pub w: u32,
    pub h: u32,
}

/// A rectangle in display-local physical pixels; `x`/`y` is the top-left corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysicalRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// Everything the core needs to know about one display.
///
/// `physical` must equal `round_half_up(logical.w * scale)` ×
/// `round_half_up(logical.h * scale)`; the macOS adapter asserts that this matches
/// the ScreenCaptureKit bitmap size and fails loudly otherwise (PITFALLS pitfall 6).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DisplayInfo {
    pub id: DisplayId,
    pub logical: LogicalRect,
    pub scale: f64,
    pub physical: PhysicalSize,
    pub primary: bool,
}

impl DisplayInfo {
    /// Global logical point → this display's physical pixel.
    ///
    /// `round_half_up((p - origin) * scale)`, clamped to `[0, w-1] × [0, h-1]`.
    pub fn to_physical(&self, p: LogicalPoint) -> PhysicalPoint {
        let x = round_half_up((p.x - self.logical.x) * self.scale);
        let y = round_half_up((p.y - self.logical.y) * self.scale);
        PhysicalPoint {
            x: clamp_axis(x, self.physical.w),
            y: clamp_axis(y, self.physical.h),
        }
    }

    /// This display's physical pixel → global logical point (`origin + p / scale`,
    /// f64, no rounding).
    pub fn to_logical(&self, p: PhysicalPoint) -> LogicalPoint {
        LogicalPoint {
            x: self.logical.x + f64::from(p.x) / self.scale,
            y: self.logical.y + f64::from(p.y) / self.scale,
        }
    }

    /// Physical bitmap size of a logical rect at `scale`:
    /// `round_half_up(w * scale) × round_half_up(h * scale)`.
    pub fn physical_size_for(logical: &LogicalRect, scale: f64) -> PhysicalSize {
        PhysicalSize {
            w: non_negative(round_half_up(logical.w * scale)),
            h: non_negative(round_half_up(logical.h * scale)),
        }
    }
}

/// Clamp a rounded pixel coordinate to `[0, extent - 1]` (an empty extent clamps to 0).
fn clamp_axis(v: i64, extent: u32) -> i32 {
    let max = i64::from(extent).saturating_sub(1).max(0);
    v.clamp(0, max) as i32
}

fn non_negative(v: i64) -> u32 {
    u32::try_from(v.max(0)).unwrap_or(u32::MAX)
}

/// Flip an AppKit `NSScreen.frame` (bottom-left origin, y-up, in points) into
/// global logical space (top-left origin, y-down) using the height of
/// `NSScreen.screens[0]` — never `NSScreen.main` (RESEARCH Q9):
/// `y_top = primary_height_pt - (y + h)`; `x`, `w`, `h` are unchanged.
pub fn flip_nsscreen_rect(primary_height_pt: f64, x: f64, y: f64, w: f64, h: f64) -> LogicalRect {
    LogicalRect {
        x,
        y: primary_height_pt - (y + h),
        w,
        h,
    }
}

/// The single rounding rule used for every logical→physical pixel mapping
/// (RESEARCH Q9 / Pattern 3): `floor(v + 0.5)`.
///
/// Rust's `f64::round` rounds half *away from zero* while JavaScript's
/// `Math.round` rounds half toward +∞; they disagree at `-0.5`. Both the Rust
/// and the TypeScript core use this exact formula so the shared fixture holds.
pub fn round_half_up(v: f64) -> i64 {
    (v + 0.5).floor() as i64
}

#[cfg(test)]
mod tests {
    use super::round_half_up;

    #[test]
    fn rounds_half_toward_positive_infinity() {
        assert_eq!(round_half_up(0.5), 1);
        assert_eq!(round_half_up(1.5), 2);
        assert_eq!(round_half_up(2.5), 3);
        assert_eq!(round_half_up(-0.5), 0);
        assert_eq!(round_half_up(-1.5), -1);
    }

    #[test]
    fn leaves_integers_and_below_half_alone() {
        assert_eq!(round_half_up(0.0), 0);
        assert_eq!(round_half_up(3.0), 3);
        assert_eq!(round_half_up(3.49), 3);
        assert_eq!(round_half_up(-3.49), -3);
        assert_eq!(round_half_up(-3.51), -4);
    }
}
