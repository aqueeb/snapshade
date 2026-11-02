//! Win32 clipboard image write (EXPORT-01; PITFALLS Pitfall 6): `CF_DIB` (24-bit, opaque,
//! bottom-up — what Word, Paint and most Win32 apps read), `CF_DIBV5` (32-bit) and the
//! registered `PNG` format (what Chromium / Slack / Teams prefer), all set in one
//! open/empty/set sequence with no delayed rendering.

use snapshade_core::capture::CaptureFrame;
use snapshade_core::encode::{encode, ImageFormat};
use snapshade_platform_api::PlatformError;
use windows::core::w;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

const CF_DIB: u32 = 8;
const CF_DIBV5: u32 = 17;

fn os(what: impl Into<String>) -> PlatformError {
    PlatformError::Os(what.into())
}

/// Copy `bytes` into a movable global block and return its handle (ownership passes to the
/// clipboard on a successful `SetClipboardData`).
unsafe fn global_from(bytes: &[u8]) -> Result<HANDLE, PlatformError> {
    // SAFETY: standard GlobalAlloc / GlobalLock / copy / GlobalUnlock sequence.
    unsafe {
        let h =
            GlobalAlloc(GMEM_MOVEABLE, bytes.len()).map_err(|e| os(format!("GlobalAlloc: {e}")))?;
        let p = GlobalLock(h);
        if p.is_null() {
            return Err(os("GlobalLock returned null"));
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p as *mut u8, bytes.len());
        // GlobalUnlock reports Err(ERROR_SUCCESS) when the lock count drops to zero.
        let _ = GlobalUnlock(h);
        Ok(HANDLE(h.0))
    }
}

fn u32le(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}
fn i32le(v: i32) -> [u8; 4] {
    v.to_le_bytes()
}
fn u16le(v: u16) -> [u8; 2] {
    v.to_le_bytes()
}

/// `BITMAPINFOHEADER` + bottom-up 24-bit BGR rows padded to 4 bytes.
fn dib24(frame: &CaptureFrame) -> Vec<u8> {
    let w = frame.width as usize;
    let h = frame.height as usize;
    let stride = (w * 3).div_ceil(4) * 4;
    let mut out = Vec::with_capacity(40 + stride * h);
    out.extend_from_slice(&u32le(40));
    out.extend_from_slice(&i32le(frame.width as i32));
    out.extend_from_slice(&i32le(frame.height as i32)); // positive = bottom-up
    out.extend_from_slice(&u16le(1));
    out.extend_from_slice(&u16le(24));
    out.extend_from_slice(&u32le(0)); // BI_RGB
    out.extend_from_slice(&u32le((stride * h) as u32));
    out.extend_from_slice(&i32le(2835)); // 72 dpi
    out.extend_from_slice(&i32le(2835));
    out.extend_from_slice(&u32le(0));
    out.extend_from_slice(&u32le(0));
    for row in (0..h).rev() {
        let src = &frame.rgba[row * w * 4..(row + 1) * w * 4];
        for px in src.as_chunks::<4>().0 {
            out.extend_from_slice(&[px[2], px[1], px[0]]);
        }
        out.resize(out.len() + (stride - w * 3), 0);
    }
    out
}

/// `BITMAPV5HEADER` (124 bytes) + bottom-up 32-bit BGRA with BI_BITFIELDS masks and sRGB.
fn dibv5(frame: &CaptureFrame) -> Vec<u8> {
    let w = frame.width as usize;
    let h = frame.height as usize;
    let mut out = Vec::with_capacity(124 + w * h * 4);
    out.extend_from_slice(&u32le(124));
    out.extend_from_slice(&i32le(frame.width as i32));
    out.extend_from_slice(&i32le(frame.height as i32));
    out.extend_from_slice(&u16le(1));
    out.extend_from_slice(&u16le(32));
    out.extend_from_slice(&u32le(3)); // BI_BITFIELDS
    out.extend_from_slice(&u32le((w * h * 4) as u32));
    out.extend_from_slice(&i32le(2835));
    out.extend_from_slice(&i32le(2835));
    out.extend_from_slice(&u32le(0));
    out.extend_from_slice(&u32le(0));
    out.extend_from_slice(&u32le(0x00FF_0000)); // red
    out.extend_from_slice(&u32le(0x0000_FF00)); // green
    out.extend_from_slice(&u32le(0x0000_00FF)); // blue
    out.extend_from_slice(&u32le(0xFF00_0000)); // alpha
    out.extend_from_slice(b"BGRs"); // LCS_sRGB ('sRGB' little-endian)
    out.resize(out.len() + 36, 0); // CIEXYZTRIPLE endpoints
    out.extend_from_slice(&u32le(0)); // gamma red
    out.extend_from_slice(&u32le(0));
    out.extend_from_slice(&u32le(0));
    out.extend_from_slice(&u32le(4)); // LCS_GM_IMAGES
    out.extend_from_slice(&u32le(0)); // profile data
    out.extend_from_slice(&u32le(0)); // profile size
    out.extend_from_slice(&u32le(0)); // reserved
    debug_assert_eq!(out.len(), 124);
    for row in (0..h).rev() {
        let src = &frame.rgba[row * w * 4..(row + 1) * w * 4];
        for px in src.as_chunks::<4>().0 {
            out.extend_from_slice(&[px[2], px[1], px[0], 255]);
        }
    }
    out
}

pub fn write_image(frame: &CaptureFrame) -> Result<(), PlatformError> {
    if frame.rgba.len() != frame.expected_len() || frame.width == 0 || frame.height == 0 {
        return Err(os("frame buffer does not match its dimensions"));
    }
    let png = encode(&frame.rgba, frame.width, frame.height, ImageFormat::Png)
        .map_err(|e| os(format!("png encode: {e}")))?;
    let dib = dib24(frame);
    let v5 = dibv5(frame);

    // SAFETY: Win32 clipboard protocol; every handle set is owned by the clipboard afterwards.
    unsafe {
        let mut opened = false;
        for _ in 0..10 {
            if OpenClipboard(None).is_ok() {
                opened = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if !opened {
            return Err(os("OpenClipboard failed (another app holds it)"));
        }
        let result = (|| -> Result<(), PlatformError> {
            EmptyClipboard().map_err(|e| os(format!("EmptyClipboard: {e}")))?;
            let png_format = RegisterClipboardFormatW(w!("PNG"));
            if png_format == 0 {
                return Err(os("RegisterClipboardFormatW(PNG) failed"));
            }
            SetClipboardData(CF_DIB, Some(global_from(&dib)?))
                .map_err(|e| os(format!("CF_DIB: {e}")))?;
            SetClipboardData(CF_DIBV5, Some(global_from(&v5)?))
                .map_err(|e| os(format!("CF_DIBV5: {e}")))?;
            SetClipboardData(png_format, Some(global_from(&png)?))
                .map_err(|e| os(format!("PNG: {e}")))?;
            Ok(())
        })();
        let _ = CloseClipboard();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::{dib24, dibv5};
    use snapshade_core::capture::CaptureFrame;

    fn frame() -> CaptureFrame {
        CaptureFrame {
            display_id: 1,
            width: 3,
            height: 2,
            scale: 1.0,
            rgba: vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, // row 0: R G B
                10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255, // row 1
            ],
        }
    }

    #[test]
    fn dib24_is_bottom_up_bgr_with_padded_rows() {
        let d = dib24(&frame());
        assert_eq!(d.len(), 40 + 12 * 2); // stride 12 (9 → 12)
        assert_eq!(&d[40..43], &[30, 20, 10]); // first stored row is image row 1, BGR
        assert_eq!(&d[52..55], &[0, 0, 255]); // then row 0 pixel 0 = red → B G R
    }

    #[test]
    fn dibv5_header_is_124_bytes_with_bitfields() {
        let d = dibv5(&frame());
        assert_eq!(u32::from_le_bytes([d[0], d[1], d[2], d[3]]), 124);
        assert_eq!(u32::from_le_bytes([d[16], d[17], d[18], d[19]]), 3);
        assert_eq!(d.len(), 124 + 3 * 2 * 4);
        assert_eq!(&d[124..128], &[30, 20, 10, 255]);
    }
}
