//! `EnumDisplayMonitors` → `DisplayTopology` (PITFALLS Pitfall 4: Per-Monitor-V2 awareness
//! must already be declared by the application manifest, or every rect comes back
//! virtualised to the primary DPI).

use std::ffi::c_void;

use snapshade_core::geometry::{DisplayInfo, LogicalRect};
use snapshade_core::topology::DisplayTopology;
use snapshade_platform_api::PlatformError;
use windows::core::BOOL;
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;

/// One monitor as Win32 reports it: physical rect in virtual-screen pixels, DPI scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawMonitor {
    pub handle: isize,
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub scale: f64,
    pub primary: bool,
}

unsafe extern "system" fn enum_proc(
    monitor: HMONITOR,
    _dc: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    // SAFETY: `data` is the `*mut Vec<RawMonitor>` passed to EnumDisplayMonitors below and
    // outlives the enumeration.
    let out = unsafe { &mut *(data.0 as *mut Vec<RawMonitor>) };
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: a correctly sized MONITORINFOEXW is a valid MONITORINFO out-pointer.
    let ok = unsafe {
        GetMonitorInfoW(
            monitor,
            &mut info as *mut MONITORINFOEXW as *mut MONITORINFO,
        )
    };
    if !ok.as_bool() {
        return BOOL(1);
    }
    let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
    // SAFETY: valid out-pointers; a failure leaves 96 (scale 1.0).
    let _ = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
    let r = info.monitorInfo.rcMonitor;
    out.push(RawMonitor {
        handle: monitor.0 as isize,
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
        scale: f64::from(dpi_x.max(1)) / 96.0,
        primary: (info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY) != 0,
    });
    BOOL(1)
}

/// Every monitor in enumeration order.
pub fn raw_monitors() -> Result<Vec<RawMonitor>, PlatformError> {
    let mut out: Vec<RawMonitor> = Vec::new();
    // SAFETY: the callback only touches the Vec behind `data` for the duration of the call.
    let ok = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(enum_proc),
            LPARAM(&mut out as *mut Vec<RawMonitor> as *mut c_void as isize),
        )
    };
    if !ok.as_bool() {
        return Err(PlatformError::Os("EnumDisplayMonitors failed".to_string()));
    }
    Ok(out)
}

/// Physical virtual-screen rects → the core's global logical space (`physical / scale`,
/// origin at the primary monitor's top-left, which Windows already places at (0, 0)).
pub fn topology_from(monitors: &[RawMonitor]) -> DisplayTopology {
    let displays = monitors
        .iter()
        .enumerate()
        .map(|(index, m)| {
            let w_px = (m.right - m.left).max(0) as u32;
            let h_px = (m.bottom - m.top).max(0) as u32;
            let logical = LogicalRect {
                x: f64::from(m.left) / m.scale,
                y: f64::from(m.top) / m.scale,
                w: f64::from(w_px) / m.scale,
                h: f64::from(h_px) / m.scale,
            };
            DisplayInfo {
                id: (index + 1) as u32,
                logical,
                scale: m.scale,
                physical: snapshade_core::geometry::PhysicalSize { w: w_px, h: h_px },
                primary: m.primary,
            }
        })
        .collect();
    DisplayTopology::new(displays)
}

pub fn snapshot() -> Result<DisplayTopology, PlatformError> {
    let monitors = raw_monitors()?;
    if monitors.is_empty() {
        return Err(PlatformError::Os("no monitors".to_string()));
    }
    Ok(topology_from(&monitors))
}
