//! Win32 window plumbing for the always-shown overlay windows (Plan 999.4-03). The overlay
//! `WebviewWindow`s are never hidden between captures: hiding and re-showing a WebView2
//! window cost 45–55 ms per press and stalled 100–650 ms one press in five (the renderer
//! resuming). Instead each window is shown once at startup and kept **permanently layered**
//! (Tauri's `set_ignore_cursor_events(true)` once, which tao turns into `WS_EX_LAYERED |
//! WS_EX_TRANSPARENT` and keeps across its own style rewrites). From then on only two things
//! ever change: the layered alpha (0 ↔ 255) and the `WS_EX_TRANSPARENT` hit-test bit — both
//! are attribute changes DWM applies in place. Toggling `WS_EX_LAYERED` itself was tried
//! first and makes Windows rebuild the window's redirection surface: the overlay showed as a
//! black rectangle until WebView2 re-presented (measured as a full black sample on both
//! monitors). tao rewrites the whole extended style from its flags on every flag change, so
//! the bits it does not know about (tool window, the alpha) are (re)applied here after
//! each such change. Everything takes a raw `HWND` as `isize`, so the app's Tauri layer needs
//! no Win32 types.

use windows::Win32::Foundation::{COLORREF, HWND};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowLongPtrW, IsIconic, IsWindow, IsWindowVisible,
    SetForegroundWindow, SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    GWL_EXSTYLE, LWA_ALPHA, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    SW_SHOWNOACTIVATE, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
};

/// Let mouse input pass through the window (`WS_EX_TRANSPARENT`) or receive it. The style
/// write is followed by the `SetWindowPos(SWP_FRAMECHANGED)` the docs require for extended
/// style changes to take effect (without it the window kept being treated as transparent:
/// it took the foreground on reveal and lost it again within a few hundred ms). The window
/// stays layered, so no surface is rebuilt. (A layered window at alpha 0 passes clicks
/// through anyway; the bit makes the idle state explicit and robust.)
pub fn set_click_through(raw: isize, through: bool) {
    let h = hwnd(raw);
    // SAFETY: plain window-style calls on a window this process owns, from its UI thread.
    unsafe {
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
        let bit = WS_EX_TRANSPARENT.0 as isize;
        let wanted = if through { ex | bit } else { ex & !bit };
        if wanted != ex {
            SetWindowLongPtrW(h, GWL_EXSTYLE, wanted);
            let _ = SetWindowPos(
                h,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
    }
}

fn hwnd(raw: isize) -> HWND {
    HWND(raw as *mut core::ffi::c_void)
}

/// Set the layered window's alpha (0 = invisible, 255 = opaque). The window must carry
/// `WS_EX_LAYERED` (tao sets it with the ignore-cursor-events flag); the attribute lives as
/// long as that bit stays set.
pub fn set_alpha(raw: isize, alpha: u8) {
    // SAFETY: plain window-attribute call on a window this process owns.
    unsafe {
        let _ = SetLayeredWindowAttributes(hwnd(raw), COLORREF(0), alpha, LWA_ALPHA);
    }
}

/// `true` when the window currently carries `WS_EX_LAYERED` (diagnostics / assertions).
pub fn is_layered(raw: isize) -> bool {
    // SAFETY: as above.
    unsafe { (GetWindowLongPtrW(hwnd(raw), GWL_EXSTYLE) & WS_EX_LAYERED.0 as isize) != 0 }
}

/// Keep the window out of Alt+Tab (`WS_EX_TOOLWINDOW`). Re-apply after any tao style change.
pub fn add_tool_window_style(raw: isize) {
    let h = hwnd(raw);
    // SAFETY: as above.
    unsafe {
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
        if ex & WS_EX_TOOLWINDOW.0 as isize == 0 {
            SetWindowLongPtrW(h, GWL_EXSTYLE, ex | WS_EX_TOOLWINDOW.0 as isize);
        }
    }
}

/// If something minimised or hid the window meanwhile (Win+D), show it again without
/// activation. `true` when that was needed.
pub fn ensure_shown_no_activate(raw: isize) -> bool {
    let h = hwnd(raw);
    // SAFETY: as above.
    unsafe {
        if !IsWindowVisible(h).as_bool() || IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
            return true;
        }
    }
    false
}

/// Second attempt at taking the foreground when tao's `set_focus` left it elsewhere: plain
/// `SetForegroundWindow` (+ `SetFocus`, which needs the owning thread — the UI thread we are
/// on). `true` when the window is the foreground afterwards.
pub fn take_foreground(raw: isize) -> bool {
    let h = hwnd(raw);
    // SAFETY: as above.
    unsafe {
        let _ = SetForegroundWindow(h);
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(h));
        GetForegroundWindow() == h
    }
}

/// The current foreground window (0 if none) — recorded right before the overlay takes the
/// foreground so [`restore_foreground`] can hand it back on dismiss.
pub fn foreground_window() -> isize {
    // SAFETY: no arguments, no preconditions.
    unsafe { GetForegroundWindow().0 as isize }
}

/// Give the foreground back to `raw` if it still exists. Only succeeds while this process
/// owns the foreground (it does, right after the overlay was dismissed); `false` otherwise.
pub fn restore_foreground(raw: isize) -> bool {
    if raw == 0 {
        return false;
    }
    let h = hwnd(raw);
    // SAFETY: as above; a stale handle is rejected by IsWindow first.
    unsafe { IsWindow(Some(h)).as_bool() && SetForegroundWindow(h).as_bool() }
}
