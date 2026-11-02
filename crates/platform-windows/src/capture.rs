//! Capture of every monitor at native pixel size, one thread per monitor: DXGI desktop
//! duplication first (`dxgi.rs`, Plan 999.4-02), GDI `BitBlt | CAPTUREBLT` into a top-down
//! 32-bit DIB as the fallback (remote sessions, rotated or HDR outputs, no frame yet);
//! BGRA → RGBA either way, the cursor composited with `DrawIconEx` when requested.

use std::ffi::c_void;
use std::time::Instant;

use snapshade_core::capture::{CaptureError, CaptureFrame, CaptureRequest, CaptureSet};
use snapshade_core::geometry::{DisplayId, PhysicalSize};
use snapshade_core::topology::DisplayTopology;
use windows::core::w;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDCW, CreateDIBSection, DeleteDC, DeleteObject, GetObjectW,
    SelectObject, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, HDC,
    HGDIOBJ, SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DrawIconEx, GetCursorInfo, GetIconInfo, CURSORINFO, CURSOR_SHOWING, DI_NORMAL, HICON, ICONINFO,
};

use crate::display::raw_monitors;
use crate::dxgi::Duplicator;
use crate::pixels::bgra_rows_to_rgba;

/// Startup (background thread): create the duplication state for every monitor of `topo`
/// and acquire its first frame, so the first hotkey press pays neither. One log line each.
pub fn prime(dup: &Duplicator, topo: &DisplayTopology) {
    let Ok(monitors) = raw_monitors() else {
        eprintln!("capture: monitor enumeration failed; nothing primed");
        return;
    };
    for info in &topo.displays {
        let Some(m) = monitors.get((info.id as usize).saturating_sub(1)) else {
            continue;
        };
        let t = Instant::now();
        match dup.prime(m.handle, (info.physical.w, info.physical.h)) {
            Ok(_) => eprintln!(
                "capture display {}: dxgi primed in {:.1} ms",
                info.id,
                t.elapsed().as_secs_f64() * 1000.0
            ),
            Err(reason) => eprintln!(
                "capture display {}: dxgi unavailable ({reason}); captures use gdi",
                info.id
            ),
        }
    }
}

/// Capture every display of `topo`: the topology ids are enumeration indices, so the
/// monitors are re-enumerated and matched by index; a mismatch in count or size is a
/// `SizeMismatch` / `DisplayFailed`, never a silently wrong frame (PITFALLS Pitfall 4).
pub fn capture_all(
    dup: &Duplicator,
    topo: &DisplayTopology,
    req: &CaptureRequest,
) -> Result<CaptureSet, CaptureError> {
    if topo.is_empty() {
        return Err(CaptureError::NoDisplays);
    }
    let monitors = raw_monitors().map_err(|e| CaptureError::DisplayFailed {
        display_id: topo.displays[0].id,
        reason: e.to_string(),
    })?;
    let include_cursor = req.include_cursor;
    let results: Vec<Result<CaptureFrame, CaptureError>> = std::thread::scope(|scope| {
        let handles: Vec<_> = topo
            .displays
            .iter()
            .map(|info| {
                let monitor = monitors.get((info.id as usize).saturating_sub(1)).copied();
                let expected = info.physical;
                let scale = info.scale;
                let display_id = info.id;
                scope.spawn(move || {
                    let m = monitor.ok_or_else(|| CaptureError::DisplayFailed {
                        display_id,
                        reason: "monitor no longer present".to_string(),
                    })?;
                    let actual = PhysicalSize {
                        w: (m.right - m.left).max(0) as u32,
                        h: (m.bottom - m.top).max(0) as u32,
                    };
                    if actual != expected {
                        return Err(CaptureError::SizeMismatch {
                            display_id,
                            expected,
                            actual,
                        });
                    }
                    let rgba = capture_one(
                        dup,
                        display_id,
                        m.handle,
                        m.left,
                        m.top,
                        actual,
                        include_cursor,
                    )?;
                    Ok(CaptureFrame {
                        display_id,
                        width: actual.w,
                        height: actual.h,
                        scale,
                        rgba,
                    })
                })
            })
            .collect();
        handles
            .into_iter()
            .zip(topo.displays.iter())
            .map(|(h, info)| {
                h.join().unwrap_or_else(|_| {
                    Err(CaptureError::DisplayFailed {
                        display_id: info.id,
                        reason: "capture thread panicked".to_string(),
                    })
                })
            })
            .collect()
    });
    let mut frames = Vec::with_capacity(results.len());
    for r in results {
        frames.push(r?);
    }
    Ok(CaptureSet::new(frames))
}

/// One monitor: DXGI into a fresh RGBA buffer, else GDI. One stderr line per monitor with
/// the backend and its timing — the `t1` breakdown the latency rows do not carry.
fn capture_one(
    dup: &Duplicator,
    display_id: DisplayId,
    hmonitor: isize,
    left: i32,
    top: i32,
    size: PhysicalSize,
    cursor: bool,
) -> Result<Vec<u8>, CaptureError> {
    let t = Instant::now();
    let mut rgba = vec![0u8; size.w as usize * size.h as usize * 4];
    match dup.capture(hmonitor, (size.w, size.h), &mut rgba) {
        Ok(timing) => {
            if cursor {
                composite_cursor_rgba(&mut rgba, size.w, size.h, left, top);
            }
            eprintln!(
                "capture display {display_id}: dxgi {:.1} ms (acquire {:.1}, copy {:.1}, readback {:.1}{})",
                t.elapsed().as_secs_f64() * 1000.0,
                timing.acquire_us as f64 / 1000.0,
                timing.copy_us as f64 / 1000.0,
                timing.readback_us as f64 / 1000.0,
                if timing.reused { ", reused" } else { "" }
            );
            Ok(rgba)
        }
        Err(reason) => {
            drop(rgba);
            let dxgi_ms = t.elapsed().as_secs_f64() * 1000.0;
            let t_gdi = Instant::now();
            let rgba = blit(display_id, left, top, size, cursor)?;
            eprintln!(
                "capture display {display_id}: dxgi unavailable after {dxgi_ms:.1} ms ({reason}); gdi {:.1} ms",
                t_gdi.elapsed().as_secs_f64() * 1000.0
            );
            Ok(rgba)
        }
    }
}

/// `BitBlt` the physical rect at (`left`, `top`) into a top-down 32-bit DIB and return RGBA.
fn blit(
    display_id: DisplayId,
    left: i32,
    top: i32,
    size: PhysicalSize,
    cursor: bool,
) -> Result<Vec<u8>, CaptureError> {
    let failed = |reason: &str| CaptureError::DisplayFailed {
        display_id,
        reason: reason.to_string(),
    };
    let w = size.w as i32;
    let h = size.h as i32;
    if w <= 0 || h <= 0 {
        return Err(failed("empty monitor rect"));
    }
    // SAFETY: plain GDI calls with handles created and released in this function.
    unsafe {
        let screen = CreateDCW(w!("DISPLAY"), None, None, None);
        if screen.is_invalid() {
            return Err(failed("CreateDCW(DISPLAY) failed"));
        }
        let mem = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let bitmap = match CreateDIBSection(Some(mem), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(b) => b,
            Err(e) => {
                let _ = DeleteDC(mem);
                let _ = DeleteDC(screen);
                return Err(failed(&format!("CreateDIBSection: {e}")));
            }
        };
        let old = SelectObject(mem, HGDIOBJ(bitmap.0));
        let blit = BitBlt(
            mem,
            0,
            0,
            w,
            h,
            Some(screen),
            left,
            top,
            SRCCOPY | CAPTUREBLT,
        );
        if blit.is_ok() && cursor {
            draw_cursor(mem, left, top);
        }
        let result = if blit.is_err() || bits.is_null() {
            Err(failed("BitBlt failed"))
        } else {
            let len = (w as usize) * (h as usize) * 4;
            let src = std::slice::from_raw_parts(bits as *const u8, len);
            let mut rgba = vec![0u8; len];
            bgra_rows_to_rgba(src, w as usize * 4, w as usize, h as usize, &mut rgba);
            Ok(rgba)
        };
        SelectObject(mem, old);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(mem);
        let _ = DeleteDC(screen);
        result
    }
}

/// The visible cursor: its icon, hotspot-corrected top-left in virtual-screen pixels, and
/// its pixel size (from the colour bitmap, else the mask — twice the height for monochrome
/// cursors). `None` when hidden. The `ICONINFO` bitmaps are released before returning.
unsafe fn cursor_shape() -> Option<(HICON, i32, i32, i32, i32)> {
    let mut ci = CURSORINFO {
        cbSize: std::mem::size_of::<CURSORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: valid out-pointer.
    if unsafe { GetCursorInfo(&mut ci) }.is_err() {
        return None;
    }
    if (ci.flags.0 & CURSOR_SHOWING.0) == 0 || ci.hCursor.is_invalid() {
        return None;
    }
    let icon = HICON(ci.hCursor.0);
    let mut ii = ICONINFO::default();
    // SAFETY: valid out-pointer; the bitmaps it returns are released below.
    if unsafe { GetIconInfo(icon, &mut ii) }.is_err() {
        return None;
    }
    let (hbm, mono) = if !ii.hbmColor.is_invalid() {
        (ii.hbmColor, false)
    } else {
        (ii.hbmMask, true)
    };
    let mut bm = BITMAP::default();
    // SAFETY: `bm` is a valid BITMAP out-buffer of the stated size.
    let got = unsafe {
        GetObjectW(
            HGDIOBJ(hbm.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut BITMAP as *mut c_void),
        )
    };
    let size = if got == 0 {
        None
    } else {
        let h = if mono { bm.bmHeight / 2 } else { bm.bmHeight };
        Some((bm.bmWidth.max(1), h.max(1)))
    };
    if !ii.hbmMask.is_invalid() {
        // SAFETY: owned by us since GetIconInfo.
        let _ = unsafe { DeleteObject(HGDIOBJ(ii.hbmMask.0)) };
    }
    if !ii.hbmColor.is_invalid() {
        let _ = unsafe { DeleteObject(HGDIOBJ(ii.hbmColor.0)) };
    }
    let (cw, ch) = size?;
    Some((
        icon,
        ci.ptScreenPos.x - ii.xHotspot as i32,
        ci.ptScreenPos.y - ii.yHotspot as i32,
        cw,
        ch,
    ))
}

/// Composite the visible cursor at its hotspot-corrected position into a DC (GDI path;
/// PITFALLS Pitfall 14).
unsafe fn draw_cursor(mem: HDC, left: i32, top: i32) {
    // SAFETY: see `cursor_shape`.
    let Some((icon, x, y, _, _)) = (unsafe { cursor_shape() }) else {
        return;
    };
    // SAFETY: valid DC and icon; 0 size draws at the icon's native size.
    let _ = unsafe { DrawIconEx(mem, x - left, y - top, icon, 0, 0, 0, None, DI_NORMAL) };
}

/// Composite the visible cursor into an RGBA frame whose top-left is virtual-screen pixel
/// (`left`, `top`) (DXGI path — the desktop image never contains the pointer). Only the
/// cursor's bounding box goes through GDI: a DIB of that box is filled from the frame,
/// `DrawIconEx` draws the cursor into it, and the box is copied back.
fn composite_cursor_rgba(rgba: &mut [u8], width: u32, height: u32, left: i32, top: i32) {
    // SAFETY: see `cursor_shape`.
    let Some((icon, x0, y0, cw, ch)) = (unsafe { cursor_shape() }) else {
        return;
    };
    let fx = x0 - left;
    let fy = y0 - top;
    let bx = fx.max(0);
    let by = fy.max(0);
    let ex = (fx + cw).min(width as i32);
    let ey = (fy + ch).min(height as i32);
    if ex <= bx || ey <= by {
        return; // cursor is on another monitor
    }
    let (bw, bh) = ((ex - bx) as usize, (ey - by) as usize);
    let stride = width as usize * 4;
    // SAFETY: plain GDI calls with handles created and released in this function; the DIB
    // bits are `bw * bh * 4` bytes for as long as the bitmap lives.
    unsafe {
        let screen = CreateDCW(w!("DISPLAY"), None, None, None);
        if screen.is_invalid() {
            return;
        }
        let mem = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: bw as i32,
                biHeight: -(bh as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let Ok(bitmap) = CreateDIBSection(Some(mem), &info, DIB_RGB_COLORS, &mut bits, None, 0)
        else {
            let _ = DeleteDC(mem);
            let _ = DeleteDC(screen);
            return;
        };
        if !bits.is_null() {
            let dib = std::slice::from_raw_parts_mut(bits as *mut u8, bw * bh * 4);
            for row in 0..bh {
                let src = (by as usize + row) * stride + bx as usize * 4;
                let dst = row * bw * 4;
                for col in 0..bw {
                    let s = src + col * 4;
                    let d = dst + col * 4;
                    dib[d] = rgba[s + 2];
                    dib[d + 1] = rgba[s + 1];
                    dib[d + 2] = rgba[s];
                    dib[d + 3] = 255;
                }
            }
            let old = SelectObject(mem, HGDIOBJ(bitmap.0));
            let _ = DrawIconEx(mem, fx - bx, fy - by, icon, 0, 0, 0, None, DI_NORMAL);
            // GdiFlush is implied by deselecting the DIB; read the box back.
            SelectObject(mem, old);
            for row in 0..bh {
                let dst = (by as usize + row) * stride + bx as usize * 4;
                let src = row * bw * 4;
                for col in 0..bw {
                    let s = src + col * 4;
                    let d = dst + col * 4;
                    rgba[d] = dib[s + 2];
                    rgba[d + 1] = dib[s + 1];
                    rgba[d + 2] = dib[s];
                    rgba[d + 3] = 255;
                }
            }
        }
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(mem);
        let _ = DeleteDC(screen);
    }
}
