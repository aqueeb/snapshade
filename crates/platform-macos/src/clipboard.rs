//! `NSPasteboard` image write (EXPORT-01; PITFALLS Pitfall 6): PNG **and** TIFF are
//! declared up front and written eagerly — no promised data — so Slack, Chrome, Word and
//! Preview each find the type they look for. The flattened screenshot is opaque, so the
//! alpha pitfalls do not apply; TIFF comes from AppKit's own encoder for the bitmap rep,
//! PNG from the `image` crate (byte-identical to the file Save writes).

use objc2::MainThreadMarker;
use objc2_app_kit::{NSPasteboard, NSPasteboardTypePNG, NSPasteboardTypeTIFF};
use objc2_foundation::{NSArray, NSData};
use snapshade_core::capture::CaptureFrame;
use snapshade_core::encode::{encode, ImageFormat};
use snapshade_platform_api::{ImageClipboard, PlatformError};

use crate::bitmap::bitmap_rep;

/// `ImageClipboard` backed by the general `NSPasteboard`. Main thread only.
#[derive(Debug, Default, Clone, Copy)]
pub struct MacClipboard;

impl ImageClipboard for MacClipboard {
    fn write_image(&self, frame: &CaptureFrame) -> Result<(), PlatformError> {
        MainThreadMarker::new().ok_or(PlatformError::MainThreadRequired)?;
        let png = encode(&frame.rgba, frame.width, frame.height, ImageFormat::Png)
            .map_err(|e| PlatformError::Os(format!("png encode: {e}")))?;
        let rep = bitmap_rep(frame)?;
        let tiff = rep
            .TIFFRepresentation()
            .ok_or_else(|| PlatformError::Os("TIFFRepresentation returned nil".to_string()))?;

        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();
        // SAFETY: the two type constants are AppKit statics; `owner` is nil (no lazy data).
        let types = unsafe { NSArray::from_slice(&[NSPasteboardTypePNG, NSPasteboardTypeTIFF]) };
        // SAFETY: documented signature; declaring with a nil owner is the eager-data form.
        unsafe { pasteboard.declareTypes_owner(&types, None) };
        let png_data = NSData::with_bytes(&png);
        // SAFETY: statics as above.
        let ok_png = unsafe { pasteboard.setData_forType(Some(&png_data), NSPasteboardTypePNG) };
        let ok_tiff = unsafe { pasteboard.setData_forType(Some(&tiff), NSPasteboardTypeTIFF) };
        if !(ok_png && ok_tiff) {
            return Err(PlatformError::Os(format!(
                "pasteboard write failed (png={ok_png}, tiff={ok_tiff})"
            )));
        }
        Ok(())
    }
}
