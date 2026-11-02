//! Screen Recording permission probe (TCC), wrapping
//! `CGPreflightScreenCaptureAccess` / `CGRequestScreenCaptureAccess` through
//! `core_graphics::access::ScreenCaptureAccess` (RESEARCH Q3; PITFALLS Pitfall 1).
//!
//! `preflight()` never prompts. `request()` opens the system prompt the first
//! time (or sends the user to System Settings) and returns the same value as
//! `preflight()`; after a grant macOS requires a FULL quit and relaunch before
//! capture works. Neither function is ever called by automation to *change*
//! state — the grant is a human action on the runner-signed binary (PKG-02).

use core_graphics::access::ScreenCaptureAccess;

/// `true` if this process (identified by bundle ID + designated requirement)
/// currently holds Screen Recording access. Never prompts.
pub fn preflight() -> bool {
    ScreenCaptureAccess.preflight()
}

/// Ask macOS for Screen Recording access. Shows the system prompt if the app is
/// not yet listed; returns the same result as [`preflight`] (false until the
/// user grants AND relaunches).
pub fn request() -> bool {
    ScreenCaptureAccess.request()
}
