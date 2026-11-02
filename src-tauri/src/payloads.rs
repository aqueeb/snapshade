//! Wire payloads shared by the macOS and Windows capture controllers: the `frame-ready`
//! event (metadata only — pixels stay behind `frame://`) and the per-generation token.

use serde::Serialize;
use snapshade_core::geometry::{DisplayId, PhysicalRect};

/// `frame-ready` payload: metadata only, the bytes stay behind `frame://`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameReady {
    pub gen: u64,
    pub display_id: DisplayId,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    /// `"native"` (macOS: the frame is on the CALayer) or `"fetch"` (Windows: the page
    /// fetches and paints it before reporting painted).
    pub fmt: String,
    /// This generation's `frame://` capability token (Plan 01-10, T-01-28): the page can
    /// read pixels only by presenting it, and it reaches the page only through this event.
    pub token: String,
    /// `true` only for self-test runs: the page fetches its frame over `frame://` AFTER
    /// reporting `overlay_painted`, proving the lazy export path end to end off the
    /// critical path. The hotkey path never sets it (D-09 point 4).
    pub fetch_after_paint: bool,
    /// SELECT-08: a rect to pre-select on this display (only when the option is on and
    /// this is the display the last selection was on).
    pub initial_selection: Option<PhysicalRect>,
    /// The persisted shared tool state the page starts from.
    pub tool: ToolPrefs,
}

/// Tool colour / marker colour / size the overlay starts with (persisted in Settings).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolPrefs {
    pub color: String,
    pub marker_color: String,
    pub size: u32,
}

impl ToolPrefs {
    pub fn from_settings(s: &snapshade_core::settings::Settings) -> Self {
        Self {
            color: s.color.clone(),
            marker_color: s.marker_color.clone(),
            size: s.tool_size,
        }
    }
}

/// `FrameReady.fmt` of the macOS path: the frame is composited natively.
pub const NATIVE_FMT: &str = "native";
/// `FrameReady.fmt` of the Windows fallback path: the page fetches the frame over
/// `frame://` and paints it first.
pub const FETCH_FMT: &str = "fetch";
/// `FrameReady.fmt` of the Windows zero-copy path (Plan 999.4-01): the display's RGBA is
/// already in the page's WebView2 shared buffer; the page paints from it.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))] // Windows handoff only
pub const SHARED_FMT: &str = "shared";

/// `additionalData` of the WebView2 `sharedbufferreceived` event (Plan 999.4-01): what the
/// page needs to match the buffer against a `frame-ready` payload before painting from it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))] // Windows handoff only; the test runs everywhere
pub struct SharedBufferInfo {
    pub display_id: DisplayId,
    /// Bitmap size in physical pixels; the buffer holds `width * height * 4` bytes.
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    /// The overlay host's label epoch the buffer was created under (diagnostics).
    pub epoch: u64,
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
impl SharedBufferInfo {
    /// The JSON WebView2 parses into `event.additionalData`.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }
}

/// A fresh capability token for one generation: 32 lowercase hex chars from two
/// `RandomState` hashers (per-process random SipHash keys from the OS) over
/// `(generation, epoch_us, display_count)`. It is an in-process capability token —
/// unguessable from the page without the `frame-ready` event that carries it — not a
/// cryptographic secret; no crate is added for it (T-01-SC).
pub fn frame_token(generation: u64, epoch_us: u128, display_count: usize) -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::BuildHasher;

    let mut token = String::with_capacity(32);
    for _ in 0..2 {
        let half = RandomState::new().hash_one((generation, epoch_us, display_count));
        token.push_str(&format!("{half:016x}"));
    }
    token
}

pub fn epoch_us() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{frame_token, SharedBufferInfo};

    #[test]
    fn shared_buffer_info_serialises_camel_case_for_the_page() {
        let info = SharedBufferInfo {
            display_id: 2,
            width: 3840,
            height: 2160,
            scale: 1.25,
            epoch: 0,
        };
        assert_eq!(
            info.to_json(),
            r#"{"displayId":2,"width":3840,"height":2160,"scale":1.25,"epoch":0}"#
        );
    }

    #[test]
    fn tokens_are_32_hex_chars_and_differ_per_call() {
        let a = frame_token(1, 1, 1);
        let b = frame_token(1, 1, 1);
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(
            a, b,
            "per-process random hasher keys make repeats astronomically unlikely"
        );
    }
}
