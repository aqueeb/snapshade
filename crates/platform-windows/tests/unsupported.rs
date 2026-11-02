//! Contract of the Windows crate on a non-Windows host (D-18): every PAL method is
//! `Unsupported`, except that permission is `AlwaysGranted` (no TCC on Windows). On a
//! Windows host the real adapters run and the smoke test below enumerates the monitors.

#[cfg(not(target_os = "windows"))]
use snapshade_core::capture::CaptureError;
use snapshade_core::capture::CaptureRequest;
#[cfg(not(target_os = "windows"))]
use snapshade_core::topology::DisplayTopology;
use snapshade_platform_api::{DisplayProvider, Permission, PlatformError, ScreenCapturer};
use snapshade_platform_windows::{WindowsCapturer, WindowsDisplayProvider};

#[cfg(not(target_os = "windows"))]
#[test]
fn display_provider_is_unsupported_off_windows() {
    assert_eq!(
        WindowsDisplayProvider.snapshot(),
        Err(PlatformError::Unsupported)
    );
}

#[cfg(not(target_os = "windows"))]
#[test]
fn capturer_is_unsupported_off_windows_but_permission_is_always_granted() {
    let capturer = WindowsCapturer::default();
    assert_eq!(capturer.permission(), Permission::AlwaysGranted);
    capturer.request_permission();
    assert_eq!(
        capturer.capture_all(
            &DisplayTopology::new(vec![]),
            &CaptureRequest::interactive()
        ),
        Err(CaptureError::Unsupported)
    );
}

/// The Windows tests below each hold a desktop duplication of every output, and Windows
/// allows one duplication per output per process (`DuplicateOutput` → `E_INVALIDARG`, after
/// blocking for seconds) — so they run one at a time.
#[cfg(target_os = "windows")]
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(target_os = "windows")]
#[test]
fn windows_enumerates_at_least_one_monitor_and_captures_it() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    // The app declares Per-Monitor-V2 awareness in its manifest; a test binary declares
    // nothing, and DXGI turns an unaware process system-DPI-aware on first use — which
    // changed the monitor rects between the two enumerations below (3072x1728 virtualised,
    // then 3840x2160 physical) when the DXGI backend landed. Declare it up front, as the app does.
    // SAFETY: plain Win32 call before any window or monitor API is used.
    let _ = unsafe {
        windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        )
    };
    let topo = WindowsDisplayProvider.snapshot().expect("monitors");
    assert!(!topo.is_empty());
    assert!(topo.displays[0].physical.w > 0);
    let _ = PlatformError::Unsupported; // keep the import used on both hosts
    let capturer = WindowsCapturer::new();
    // Prime first (the hotkey path never creates a duplication itself, Plan 999.4-04), then
    // capture twice on the primed states — or on GDI, if the compositor presented nothing
    // within the priming window.
    capturer.prime(&topo);
    for _ in 0..2 {
        let set = capturer
            .capture_all(&topo, &CaptureRequest::interactive())
            .expect("screen capture");
        assert_eq!(set.frames.len(), topo.displays.len());
        assert_eq!(set.frames[0].rgba.len(), set.frames[0].expected_len());
    }
    assert_eq!(capturer.permission(), Permission::AlwaysGranted);
}

/// Plan 999.4-04: a press during startup priming must not wait for it. `Duplicator::prime`
/// used to hold each monitor's lock for its whole ~5 s retry loop (and `state_for` blocked
/// under the global map lock behind it); now priming locks per 100 ms attempt, the capture
/// path takes a state with `try_lock` (GDI when busy) and device creation runs outside the
/// map lock. The bound is generous: two 4K GDI blits are about 200 ms.
#[cfg(target_os = "windows")]
#[test]
fn capture_during_priming_does_not_wait_for_it() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    // SAFETY: plain Win32 call before any window or monitor API is used (see above).
    let _ = unsafe {
        windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        )
    };
    let topo = WindowsDisplayProvider.snapshot().expect("monitors");
    let capturer = std::sync::Arc::new(WindowsCapturer::new());
    let primer = {
        let capturer = std::sync::Arc::clone(&capturer);
        let topo = topo.clone();
        std::thread::spawn(move || capturer.prime(&topo))
    };
    std::thread::sleep(std::time::Duration::from_millis(20));
    let t = std::time::Instant::now();
    let set = capturer
        .capture_all(&topo, &CaptureRequest::interactive())
        .expect("screen capture during priming");
    let elapsed = t.elapsed();
    assert_eq!(set.frames.len(), topo.displays.len());
    assert!(
        elapsed < std::time::Duration::from_millis(1000),
        "capture during priming took {elapsed:?}"
    );
    primer.join().expect("prime thread");
}
