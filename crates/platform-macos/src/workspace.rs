//! `NSWorkspace`: reveal a saved file in Finder (EXPORT-06 click) and open URLs — the
//! `x-apple.systempreferences:` Screen Recording deep link (CAPTURE-03) and Help.

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSWorkspace};
use objc2_foundation::{NSArray, NSString, NSURL};
use snapshade_platform_api::{PlatformError, Shell};
use std::path::Path;

/// `Shell` backed by `NSWorkspace`. Main thread only.
#[derive(Debug, Default, Clone, Copy)]
pub struct MacShell;

impl Shell for MacShell {
    fn reveal_file(&self, path: &Path) -> Result<(), PlatformError> {
        MainThreadMarker::new().ok_or(PlatformError::MainThreadRequired)?;
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        let urls = NSArray::from_retained_slice(&[url]);
        NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(&urls);
        Ok(())
    }

    fn open_url(&self, url: &str) -> Result<(), PlatformError> {
        MainThreadMarker::new().ok_or(PlatformError::MainThreadRequired)?;
        let ns_url = NSURL::URLWithString(&NSString::from_str(url))
            .ok_or_else(|| PlatformError::Os(format!("invalid URL: {url}")))?;
        if NSWorkspace::sharedWorkspace().openURL(&ns_url) {
            Ok(())
        } else {
            Err(PlatformError::Os(format!("openURL refused: {url}")))
        }
    }
}

/// Bring Snapshade to the front so a regular window (Options, About, permission
/// guidance) can become key while the app runs under the Accessory activation policy.
/// Main thread only.
pub fn activate_app() -> Result<(), PlatformError> {
    let mtm = MainThreadMarker::new().ok_or(PlatformError::MainThreadRequired)?;
    let app = NSApplication::sharedApplication(mtm);
    // macOS 14+ `activate()` is cooperative: it is refused unless the frontmost app yields
    // or the request is user-initiated. A window opened from a notification click or from
    // automation never qualifies, so the legacy forced form is requested first (still
    // honoured in practice; a no-op at worst) and the cooperative one after it.
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    app.activate();
    Ok(())
}

/// Deep link to the Screen Recording pane of System Settings (CAPTURE-03).
pub const SCREEN_RECORDING_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";
