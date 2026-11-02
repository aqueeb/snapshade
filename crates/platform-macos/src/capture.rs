//! Per-display native-resolution capture with `SCScreenshotManager`
//! (RESEARCH Q3; PITFALLS Pitfall 2 and 6; CLAUDE.md "What NOT to Use").
//!
//! The only capture API is ScreenCaptureKit's `SCScreenshotManager::capture_image`
//! (macOS 14+). The deprecated CoreGraphics `CGDisplayCreateImage*` /
//! `CGWindowListCreateImage` family and the `xcap` crate are never used.
//!
//! `SCShareableContent::get()` enumerates every window and is slow, so it runs
//! once at construction (only if permission is already granted — without it the
//! call errors) and on [`MacCapturer::refresh`]; never on the hotkey path.
//! `capture_image` blocks the calling thread until Apple's completion handler
//! fires, so [`MacCapturer::capture_native`] runs one scoped thread per display
//! (N displays cost max(t), not sum(t)) and must never be called on the main
//! thread, which has to stay free to order the overlay panels front.
//!
//! Plan B (Plan 01-09; RESEARCH Q5 line 461): the hotkey path keeps the `CGImage`
//! ScreenCaptureKit returns — [`NativeFrame`] — and hands it to the overlay's native
//! `FrameLayer` zero-copy. The RGBA conversion (`rgba_data()`, one ~24–64 MB bitmap
//! draw per display) is no longer part of the capture: [`NativeFrame::to_capture_frame`]
//! performs it lazily, only when a consumer asks `frame://` for pixels (Phase 2's
//! selection crop / export). [`ScreenCapturer::capture_all`] stays the eager trait
//! contract (`capture_native` + `to_capture_frame` per frame + `CaptureSet::new`) for
//! `FakeCapturer`-shaped callers.
//!
//! **Self-exclusion (Plan 01-10; ROADMAP SC2; 01-SPIKE-RESULT open issue 1).** "No overlay
//! chrome ever lands in Snapshade's own capture" rests on two things: capture-then-show
//! (the panels are hidden while the hotkey capture runs) **and** an `SCContentFilter`
//! built with `with_excluding_applications(&[Snapshade], &[])`, so a capture taken while
//! the overlay IS visible (the `exclusion` self-test today; an instant-fullscreen hotkey
//! in a later phase) still excludes every Snapshade window — the native frame layer, the
//! dim, the probe input. It deliberately does NOT rest on the overlay windows' AppKit
//! `sharingType`: Plan 01-07 set `NSWindowSharingNone` and the 01-08 hardware session
//! found that DisplayLink Manager renders its virtual displays by *capturing* them, so a
//! sharing-excluded window is simply invisible on a DisplayLink monitor. Panels therefore
//! keep the default sharing type and the exclusion is a capture-side filter. When
//! [`SelfExclusion::Exclude`] is requested and the running application cannot be found in
//! `SCShareableContent` (after one refresh), the capture fails closed with
//! `self-exclusion unavailable` rather than risk overlay chrome in the result.
//!
//! Byte order: `to_capture_frame` always produces RGBA, the `CaptureFrame` contract.

use std::sync::Mutex;

use screencapturekit::prelude::{
    SCContentFilter, SCDisplay, SCRunningApplication, SCShareableContent, SCStreamConfiguration,
};
use screencapturekit::screenshot_manager::{CGImage, CGImageExt, SCScreenshotManager};
use screencapturekit::shareable_content::SCShareableContentInfo;
use snapshade_core::capture::{CaptureError, CaptureFrame, CaptureRequest, CaptureSet};
use snapshade_core::geometry::{DisplayId, DisplayInfo, PhysicalSize};
use snapshade_core::topology::DisplayTopology;
use snapshade_platform_api::{Permission, ScreenCapturer};

use crate::permission::{preflight, request};

/// Whether a capture excludes Snapshade's own application through the `SCContentFilter`
/// (ROADMAP SC2). The hotkey path always uses [`SelfExclusion::Exclude`];
/// [`SelfExclusion::Include`] exists only as the control arm of the `exclusion`
/// self-test, which proves the flag changes the result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfExclusion {
    /// Build the filter with `with_excluding_applications(&[self], &[])`; fail closed
    /// (`self-exclusion unavailable`) if the app is not in `SCShareableContent`.
    Exclude,
    /// Plain `with_display` filter — Snapshade's visible windows ARE captured.
    Include,
}

/// Reason of the fail-closed `DisplayFailed` when the app cannot be excluded.
pub const SELF_EXCLUSION_UNAVAILABLE: &str =
    "self-exclusion unavailable: Snapshade not in SCShareableContent applications";

/// `ScreenCapturer` backed by ScreenCaptureKit.
#[derive(Default)]
pub struct MacCapturer {
    /// Prefetched shareable content; `None` until permission is granted and a
    /// fetch succeeded. `SCDisplay`s are re-derived from it per capture.
    content: Mutex<Option<SCShareableContent>>,
}

impl MacCapturer {
    /// Build the capturer and prefetch `SCShareableContent` if Screen Recording
    /// is already granted (RESEARCH Q3 line 347: without permission `get()` errors).
    pub fn new() -> Self {
        let capturer = Self {
            content: Mutex::new(None),
        };
        if preflight() {
            capturer.refresh();
        }
        capturer
    }

    /// Re-fetch `SCShareableContent` (display change, or after the first grant).
    /// Returns `true` on success; failures are logged to stderr and leave the
    /// previous content in place.
    pub fn refresh(&self) -> bool {
        match SCShareableContent::get() {
            Ok(content) => {
                *self.content.lock().unwrap_or_else(|p| p.into_inner()) = Some(content);
                true
            }
            Err(err) => {
                eprintln!("shareable content fetch failed: {err}");
                false
            }
        }
    }

    /// Current `SCDisplay` list from the prefetched content (fresh retained
    /// handles each call). Empty if nothing has been fetched yet.
    fn sc_displays(&self) -> Vec<SCDisplay> {
        self.content
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .map(SCShareableContent::displays)
            .unwrap_or_default()
    }

    /// This process in the prefetched content's `applications()` (matched by
    /// `process_id`). `None` if nothing has been fetched yet or the app is not listed.
    fn self_application_prefetched(&self) -> Option<SCRunningApplication> {
        let pid = i32::try_from(std::process::id()).ok()?;
        self.content
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .and_then(|content| {
                content
                    .applications()
                    .into_iter()
                    .find(|app| app.process_id() == pid)
            })
    }

    /// The running Snapshade application as ScreenCaptureKit sees it — the handle
    /// `with_excluding_applications` needs. Looks in the prefetched content first; if
    /// the app is absent (the launch-time fetch ran before the panels existed), calls
    /// [`MacCapturer::refresh`] once and looks again (slow path: at most the first press
    /// after launch, before the post-prepare refresh has landed). `None` means the app is
    /// genuinely not listed and the caller must fail closed.
    pub fn self_application(&self) -> Option<SCRunningApplication> {
        if let Some(app) = self.self_application_prefetched() {
            return Some(app);
        }
        if self.refresh() {
            return self.self_application_prefetched();
        }
        None
    }
}

impl ScreenCapturer for MacCapturer {
    fn permission(&self) -> Permission {
        if preflight() {
            Permission::Granted
        } else {
            Permission::Denied
        }
    }

    fn request_permission(&self) {
        request();
    }

    /// Eager contract: [`MacCapturer::capture_native`] (self-excluding, like the hotkey
    /// path) followed by one RGBA conversion per frame, sorted by display id by
    /// `CaptureSet::new`. Any per-display failure aborts the whole call (no partial `Ok`).
    fn capture_all(
        &self,
        topo: &DisplayTopology,
        req: &CaptureRequest,
    ) -> Result<CaptureSet, CaptureError> {
        let native = self.capture_native(topo, req, SelfExclusion::Exclude)?;
        let mut frames = Vec::with_capacity(native.len());
        for frame in &native {
            frames.push(frame.to_capture_frame()?);
        }
        Ok(CaptureSet::new(frames))
    }
}

/// One display's frozen frame as ScreenCaptureKit produced it: the retained `CGImage`
/// (Core Foundation reference-counted, `Send + Sync`) plus the geometry the overlay and
/// the `frame://` responder need. No pixel copy has been made yet.
#[derive(Debug, Clone)]
pub struct NativeFrame {
    pub display_id: DisplayId,
    /// Bitmap width in physical pixels (== the display's SCK pixel size, asserted).
    pub width: u32,
    /// Bitmap height in physical pixels.
    pub height: u32,
    /// Pixels per point of the display (`backingScaleFactor`).
    pub scale: f64,
    image: CGImage,
}

impl NativeFrame {
    /// The retained `CGImage` — what `FrameLayer::set_contents` installs zero-copy.
    pub fn image(&self) -> &CGImage {
        &self.image
    }

    /// Convert to the RGBA [`CaptureFrame`] contract (one bitmap-context draw, ~24–64 MB
    /// per display). Lazy by design: never called on the hotkey path, only when a
    /// consumer asks `frame://` for pixels. Fails with `DisplayFailed` on a conversion
    /// error or a buffer whose length is not `width * height * 4`.
    pub fn to_capture_frame(&self) -> Result<CaptureFrame, CaptureError> {
        let display_id = self.display_id;
        let failed = |reason: String| CaptureError::DisplayFailed { display_id, reason };
        let rgba = self
            .image
            .rgba_data()
            .map_err(|e| failed(format!("rgba_data: {e}")))?;
        let frame = CaptureFrame {
            display_id,
            width: self.width,
            height: self.height,
            scale: self.scale,
            rgba,
        };
        if frame.rgba.len() != frame.expected_len() {
            return Err(failed(format!(
                "rgba buffer is {} bytes, expected {}",
                frame.rgba.len(),
                frame.expected_len()
            )));
        }
        Ok(frame)
    }
}

impl MacCapturer {
    /// Capture every display of `topo` concurrently (one scoped thread per display,
    /// `SCScreenshotManager::capture_image` at the SCK pixel size) and return the
    /// retained `CGImage`s with NO RGBA conversion — the Plan B hotkey path. Same
    /// error semantics as the trait: `NoDisplays`, `PermissionDenied`, `DisplayFailed`
    /// (no SCDisplay, filter/capture error, thread panic, or — with
    /// [`SelfExclusion::Exclude`] — the app missing from `SCShareableContent`, reported
    /// for the first display as [`SELF_EXCLUSION_UNAVAILABLE`] before any capture starts),
    /// `SizeMismatch`; any per-display failure aborts the whole call. The result is sorted
    /// by display id (mirrors `CaptureSet::new`) regardless of thread completion order.
    ///
    /// Must not be called on the main thread (`capture_image` blocks until Apple's
    /// completion handler fires).
    pub fn capture_native(
        &self,
        topo: &DisplayTopology,
        req: &CaptureRequest,
        exclusion: SelfExclusion,
    ) -> Result<Vec<NativeFrame>, CaptureError> {
        if topo.is_empty() {
            return Err(CaptureError::NoDisplays);
        }
        if !preflight() {
            return Err(CaptureError::PermissionDenied);
        }
        let mut sc_displays = self.sc_displays();
        if sc_displays.is_empty() && self.refresh() {
            sc_displays = self.sc_displays();
        }

        // Pair each display of the topology with its SCDisplay before spawning so a
        // missing display is reported without starting any capture.
        let mut jobs: Vec<(DisplayInfo, SCDisplay)> = Vec::with_capacity(topo.displays.len());
        for info in &topo.displays {
            let position = sc_displays
                .iter()
                .position(|d| d.display_id() == info.id)
                .ok_or_else(|| CaptureError::DisplayFailed {
                    display_id: info.id,
                    reason: "no SCDisplay".to_string(),
                })?;
            jobs.push((*info, sc_displays.swap_remove(position)));
        }

        // SC2: resolve the application to exclude ONCE, before any thread starts, and
        // fail closed if it cannot be found — never capture without the filter while a
        // panel could be on screen.
        let excluded_app: Option<SCRunningApplication> = match exclusion {
            SelfExclusion::Exclude => {
                Some(
                    self.self_application()
                        .ok_or_else(|| CaptureError::DisplayFailed {
                            display_id: topo.displays[0].id,
                            reason: SELF_EXCLUSION_UNAVAILABLE.to_string(),
                        })?,
                )
            }
            SelfExclusion::Include => None,
        };
        let excluded_app = excluded_app.as_ref();

        let include_cursor = req.include_cursor;
        let results: Vec<Result<NativeFrame, CaptureError>> = std::thread::scope(|scope| {
            let handles: Vec<_> = jobs
                .iter()
                .map(|(info, display)| {
                    let display_id = info.id;
                    let handle = scope
                        .spawn(move || capture_one(info, display, include_cursor, excluded_app));
                    (display_id, handle)
                })
                .collect();
            handles
                .into_iter()
                .map(|(display_id, handle)| {
                    handle.join().unwrap_or_else(|_| {
                        Err(CaptureError::DisplayFailed {
                            display_id,
                            reason: "capture thread panicked".to_string(),
                        })
                    })
                })
                .collect()
        });

        let mut frames = Vec::with_capacity(results.len());
        for result in results {
            frames.push(result?);
        }
        frames.sort_by_key(|f| f.display_id);
        Ok(frames)
    }
}

/// The RESEARCH Q3 recipe for one display, run on a worker thread. Returns the retained
/// `CGImage`; no pixel conversion happens here (Plan B). `excluded_app` is Snapshade
/// itself on the hotkey path (SC2 filter) and `None` only for the self-test's control
/// capture.
fn capture_one(
    info: &DisplayInfo,
    display: &SCDisplay,
    include_cursor: bool,
    excluded_app: Option<&SCRunningApplication>,
) -> Result<NativeFrame, CaptureError> {
    let display_id = info.id;
    let failed = |reason: String| CaptureError::DisplayFailed { display_id, reason };

    let filter = SCContentFilter::create().with_display(display);
    let filter = match excluded_app {
        // Everything on the display except Snapshade's own windows (RESEARCH Q3 line 350,
        // now the one and only exclusion mechanism — see the module doc).
        Some(app) => filter.with_excluding_applications(&[app], &[]),
        None => filter,
    };
    let filter = filter
        .build()
        .map_err(|e| failed(format!("content filter: {e}")))?;

    // Native pixel size from SCK — NOT points (Pitfall 6: half-resolution Retina).
    let sc_info = SCShareableContentInfo::for_filter(&filter)
        .ok_or_else(|| failed("no SCShareableContentInfo for filter".to_string()))?;
    let (pw, ph) = sc_info.pixel_size();
    let actual = PhysicalSize { w: pw, h: ph };
    if actual != info.physical {
        return Err(CaptureError::SizeMismatch {
            display_id,
            expected: info.physical,
            actual,
        });
    }
    let point_pixel_scale = f64::from(sc_info.point_pixel_scale());
    if (point_pixel_scale - info.scale).abs() > 1e-6 {
        eprintln!(
            "display {display_id}: SCK point_pixel_scale {point_pixel_scale} != backingScaleFactor {}",
            info.scale
        );
    }

    let config = SCStreamConfiguration::new()
        .with_width(pw)
        .with_height(ph)
        .with_shows_cursor(include_cursor);

    // Blocking until Apple's completion handler fires; never on the main thread.
    let image = SCScreenshotManager::capture_image(&filter, &config)
        .map_err(|e| failed(format!("capture_image: {e}")))?;
    let image_size = PhysicalSize {
        w: u32::try_from(image.width()).unwrap_or(u32::MAX),
        h: u32::try_from(image.height()).unwrap_or(u32::MAX),
    };
    if image_size != actual {
        return Err(CaptureError::SizeMismatch {
            display_id,
            expected: actual,
            actual: image_size,
        });
    }

    Ok(NativeFrame {
        display_id,
        width: pw,
        height: ph,
        scale: info.scale,
        image,
    })
}
