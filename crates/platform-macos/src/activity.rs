//! `NSProcessInfo` activity for the idle-on-battery scenario (RESEARCH Q6 line 472,
//! PITFALLS Pitfall 12; Plan 01-07 Task 3).
//!
//! An `LSUIElement` accessory app is opted out of App Nap by the system (WWDC 2013),
//! so this is a MEASURED mitigation, not a default: the composition root holds the
//! token only when `SNAPSHADE_ACTIVITY=1`, and Plan 01-08 runs scenario (c) with and
//! without it. `endActivity:` is never called — the token lives for the process.

use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};

/// The opaque activity object returned by `beginActivityWithOptions:reason:`. The
/// activity ends when this token is released (process exit, in practice).
pub struct ActivityToken {
    /// Held only for its release on drop; never messaged.
    _activity: Retained<ProtocolObject<dyn NSObjectProtocol>>,
}

// SAFETY: the token is never messaged after creation; it is only retained here and
// released on drop. `NSProcessInfo` and its activity objects are documented as
// thread-safe, so holding/releasing the token from any thread (Tauri's managed
// state is dropped wherever the app exits) is sound.
unsafe impl Send for ActivityToken {}
unsafe impl Sync for ActivityToken {}

impl std::fmt::Debug for ActivityToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ActivityToken(UserInitiated | LatencyCritical)")
    }
}

/// Begin a `UserInitiated | LatencyCritical` activity with `reason` and return the
/// token that keeps it alive. Callable from any thread.
pub fn begin_latency_critical_activity(reason: &str) -> ActivityToken {
    let options = NSActivityOptions::UserInitiated | NSActivityOptions::LatencyCritical;
    let token = NSProcessInfo::processInfo()
        .beginActivityWithOptions_reason(options, &NSString::from_str(reason));
    ActivityToken { _activity: token }
}
