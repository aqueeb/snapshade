//! RGBA8 → `NSBitmapImageRep` (shared by the pasteboard and print adapters).

use std::ptr;

use objc2::rc::Retained;
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{NSBitmapFormat, NSBitmapImageRep, NSCalibratedRGBColorSpace, NSImage};
use objc2_foundation::NSSize;
use snapshade_core::capture::CaptureFrame;
use snapshade_platform_api::PlatformError;

fn os(what: &str) -> PlatformError {
    PlatformError::Os(format!("bitmap: {what}"))
}

/// Build a non-premultiplied 32-bit RGBA `NSBitmapImageRep` holding a copy of `frame`'s
/// pixels. Fails (never panics) on a malformed buffer.
pub fn bitmap_rep(frame: &CaptureFrame) -> Result<Retained<NSBitmapImageRep>, PlatformError> {
    if frame.width == 0 || frame.height == 0 || frame.rgba.len() != frame.expected_len() {
        return Err(os("frame buffer does not match its dimensions"));
    }
    let width = frame.width as isize;
    let height = frame.height as isize;
    let bytes_per_row = width * 4;
    // SAFETY: a null planes pointer asks AppKit to allocate the backing store; the
    // colour-space name is an AppKit static; all sizes are consistent with 4 bytes/px.
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bitmapFormat_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            ptr::null_mut(),
            width,
            height,
            8,
            4,
            true,
            false,
            NSCalibratedRGBColorSpace,
            NSBitmapFormat::AlphaNonpremultiplied,
            bytes_per_row,
            32,
        )
    }
    .ok_or_else(|| os("NSBitmapImageRep init returned nil"))?;
    let dst = rep.bitmapData();
    if dst.is_null() {
        return Err(os("bitmapData is null"));
    }
    // SAFETY: AppKit allocated `height * bytes_per_row` bytes at `dst` (we asked for a
    // tightly packed layout: bytesPerRow = width * 4), and the source has exactly that many.
    unsafe { ptr::copy_nonoverlapping(frame.rgba.as_ptr(), dst, frame.rgba.len()) };
    Ok(rep)
}

/// An `NSImage` of `frame` whose point size equals its pixel size (1 px = 1 pt), for views
/// that scale the image themselves (print).
pub fn ns_image(
    frame: &CaptureFrame,
    _mtm: MainThreadMarker,
) -> Result<Retained<NSImage>, PlatformError> {
    let rep = bitmap_rep(frame)?;
    let image = NSImage::initWithSize(
        NSImage::alloc(),
        NSSize::new(f64::from(frame.width), f64::from(frame.height)),
    );
    image.addRepresentation(&rep);
    Ok(image)
}
