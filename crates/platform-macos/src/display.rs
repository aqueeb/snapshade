//! `NSScreen` → [`DisplayTopology`] (RESEARCH Q4 lines 378-386, Q9).
//!
//! AppKit reports every screen frame in points with a bottom-left origin whose
//! zero is the bottom-left of `NSScreen.screens[0]` (the menu-bar screen). The
//! adapter flips each frame ONCE with [`flip_nsscreen_rect`] using
//! `screens[0].frame.height` — never `NSScreen.main`, which follows the key
//! window — into the core's top-left, y-down global logical space.
//!
//! [`raw_screens`] exposes the unflipped AppKit frames for overlay placement:
//! `src-tauri/src/overlay.rs` sets each `NSWindow` frame to its `NSScreen` frame
//! verbatim, bypassing tao's monitor positions (tauri#7890).
//!
//! [`observe_screen_changes`] registers for
//! `NSApplicationDidChangeScreenParametersNotification` (hot-plug, resolution or
//! arrangement change — RESEARCH Q4 line 416, ARCHITECTURE Pattern 3) so the
//! composition root can re-snapshot and re-prepare the overlays.

use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplicationDidChangeScreenParametersNotification, NSScreen};
use objc2_foundation::{ns_string, NSNotification, NSNotificationCenter, NSNumber};
use snapshade_core::geometry::{flip_nsscreen_rect, DisplayId, DisplayInfo};
use snapshade_core::topology::DisplayTopology;
use snapshade_platform_api::{DisplayProvider, PlatformError};

/// One `NSScreen` as AppKit reports it: `frame` in points, y-up, origin relative
/// to the bottom-left of `screens[0]`; `scale` is `backingScaleFactor`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawScreen {
    /// `CGDirectDisplayID` from `deviceDescription["NSScreenNumber"]` — the join
    /// key with ScreenCaptureKit's `SCDisplay::display_id()`.
    pub id: DisplayId,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub scale: f64,
}

/// Enumerate `NSScreen::screens` in AppKit order (index 0 is the menu-bar screen).
///
/// Must run on the main thread; returns `Err(MainThreadRequired)` otherwise.
pub fn raw_screens() -> Result<Vec<RawScreen>, PlatformError> {
    let mtm = MainThreadMarker::new().ok_or(PlatformError::MainThreadRequired)?;
    let screens = NSScreen::screens(mtm);
    let mut out = Vec::with_capacity(screens.len());
    for screen in screens.iter() {
        let frame = screen.frame();
        let scale = screen.backingScaleFactor();
        let id = screen_number(&screen)?;
        out.push(RawScreen {
            id,
            x: frame.origin.x,
            y: frame.origin.y,
            w: frame.size.width,
            h: frame.size.height,
            scale,
        });
    }
    Ok(out)
}

/// Read `deviceDescription["NSScreenNumber"]` as a `CGDirectDisplayID`.
fn screen_number(screen: &NSScreen) -> Result<DisplayId, PlatformError> {
    let description = screen.deviceDescription();
    let value = description
        .objectForKey(ns_string!("NSScreenNumber"))
        .ok_or_else(|| PlatformError::Os("NSScreen has no NSScreenNumber".to_string()))?;
    let number: Retained<NSNumber> = value
        .downcast::<NSNumber>()
        .map_err(|_| PlatformError::Os("NSScreenNumber is not an NSNumber".to_string()))?;
    Ok(number.unsignedIntValue())
}

/// `DisplayProvider` backed by `NSScreen`.
#[derive(Debug, Default, Clone, Copy)]
pub struct MacDisplayProvider;

impl DisplayProvider for MacDisplayProvider {
    /// Snapshot every screen into core types. `screens[0]` is marked primary and
    /// supplies the flip height; `physical` is `physical_size_for(logical, scale)`
    /// and is asserted against the ScreenCaptureKit pixel size at capture time
    /// (Pitfall 6).
    fn snapshot(&self) -> Result<DisplayTopology, PlatformError> {
        let screens = raw_screens()?;
        let primary_height_pt = screens.first().map(|s| s.h).unwrap_or(0.0);
        let displays = screens
            .iter()
            .enumerate()
            .map(|(index, s)| {
                let logical = flip_nsscreen_rect(primary_height_pt, s.x, s.y, s.w, s.h);
                DisplayInfo {
                    id: s.id,
                    logical,
                    scale: s.scale,
                    physical: DisplayInfo::physical_size_for(&logical, s.scale),
                    primary: index == 0,
                }
            })
            .collect();
        Ok(DisplayTopology::new(displays))
    }
}

/// Registration token for [`observe_screen_changes`]; unregisters on drop.
pub struct ScreenObserver {
    token: Retained<ProtocolObject<dyn NSObjectProtocol>>,
}

// SAFETY: the token is only ever handed back to `NSNotificationCenter::removeObserver`,
// which is documented thread-safe; it is never messaged otherwise.
unsafe impl Send for ScreenObserver {}
unsafe impl Sync for ScreenObserver {}

impl std::fmt::Debug for ScreenObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ScreenObserver(NSApplicationDidChangeScreenParametersNotification)")
    }
}

impl Drop for ScreenObserver {
    fn drop(&mut self) {
        let observer: &AnyObject = (*self.token).as_ref();
        // SAFETY: `token` is the object `addObserverForName:…` returned; removing it
        // from the default center is the documented way to end the observation.
        unsafe { NSNotificationCenter::defaultCenter().removeObserver(observer) };
    }
}

/// Call `callback` every time AppKit posts
/// `NSApplicationDidChangeScreenParametersNotification`. The block runs on the
/// posting thread (the main thread for this notification), so the callback may
/// touch AppKit directly; it should return quickly. Must be registered on the main
/// thread. The notification fires in bursts for one physical change — coalesce in
/// the callback.
pub fn observe_screen_changes(
    callback: Box<dyn Fn() + Send + Sync>,
) -> Result<ScreenObserver, PlatformError> {
    MainThreadMarker::new().ok_or(PlatformError::MainThreadRequired)?;
    let block = RcBlock::new(move |_notification: NonNull<NSNotification>| callback());
    // SAFETY: reading an extern static notification name; AppKit defines it.
    let name = unsafe { NSApplicationDidChangeScreenParametersNotification };
    // SAFETY: no object filter, no queue (deliver on the posting thread); the block is
    // `Send` (it owns a `Send + Sync` callback) and is retained by the center.
    let token = unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(name),
            None,
            None,
            &block,
        )
    };
    Ok(ScreenObserver { token })
}
