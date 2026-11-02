//! `SNAPSHADE_SELFTEST` modes — automated proofs of the capture path that run in
//! the real signed binary (D-09 instrumentation seed; RESEARCH Q12).
//!
//! * `preflight` — prints `preflight: granted` (exit 0) or `preflight: denied`
//!   (exit 3) from the real TCC probe and never prompts. Either answer is a valid
//!   outcome: the grant itself is a human action on this runner-signed binary
//!   (PKG-02), never something automation performs.
//! * `<N>` (N >= 1) — runs the exact hotkey path N times after setup (panels
//!   created, webviews loaded), spaced 1000 ms apart; each run waits for
//!   `overlay_painted` from every display, then (Plan 01-10 Task 2, the lazy-fetch
//!   proof) for one `overlay_frame_fetched` per display — the page fetches its frame over
//!   `frame://` AFTER reporting painted, off the critical path, and must get
//!   `width × height × 4` bytes with the current token — and only then hides the panels.
//!   Rows are tagged `selftest` with `selftest: true` (D-10 honesty). Prints
//!   `selftest ok runs=<N> median_total_ms=<m> displays=<k> lazy_fetch=ok` (exit 0);
//!   otherwise `selftest failed: run <i>: <reason>` (exit 1; a fetch problem reads
//!   `lazy fetch …`). The first run on a Mac without the grant fails with the
//!   `permission missing` reason — the designed human check.
//! * `exclusion` (Plan 01-10; ROADMAP SC2; T-01-27) — the self-exclusion proof. One
//!   self-test run brings the overlay up (native frame + 50 % dim on every display),
//!   then, WITH the overlay visible: capture **A** = the frames already in the
//!   `FrameStore` (the pre-show capture), capture **B** = a fresh capture with
//!   `SelfExclusion::Exclude` (what the hotkey path uses), capture **C** = a fresh
//!   capture with `SelfExclusion::Include` (control: the dimmed overlay IS in it). The
//!   panels are hidden and the mean luminance (`0.2126 R + 0.7152 G + 0.0722 B` over
//!   every 97th pixel) of A, B and C is compared per display:
//!   - `mean(A) < 40` → `selftest exclusion inconclusive: display <id> mean <m> < 40
//!     (desktop too dark)`, exit **4** (open a light window and re-run);
//!   - `|mean(B) − mean(A)| > 8` → `selftest exclusion failed: display <id>: excluded
//!     capture differs from the pre-show capture by <d> (overlay leaked into the
//!     capture)`, exit **1**;
//!   - `mean(C) > 0.75 × mean(A)` → `selftest exclusion failed: display <id>: control
//!     capture is not darker (ratio <r>) — the exclusion flag has no effect`, exit **1**;
//!   - otherwise `selftest exclusion ok displays=<k> max_delta_excluded=<d.d>
//!     max_control_ratio=<r.rr>`, exit **0**.
//!
//!   The control arm makes the proof non-vacuous: B ≈ A alone would also hold if the
//!   filter did nothing and the overlay happened to be invisible; C must be darker.
//!
//!   The run this mode triggers is a `selftest` row like any other (D-09/D-10:
//!   excluded from gated stats by default); its page also performs the lazy fetch, whose
//!   `overlay_frame_fetched` report is simply dropped here (no `FetchReporter` installed).
//!
//! Results go to stdout; everything else the app prints goes to stderr.

use std::collections::BTreeMap;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use snapshade_core::capture::{CaptureFrame, CaptureRequest};
use snapshade_core::geometry::DisplayId;
use snapshade_platform_api::OverlayHost;
use snapshade_platform_macos::SelfExclusion;
use tauri::{AppHandle, Manager, Wry};

use crate::capture_controller::{trigger, CaptureState, Trigger, TriggerSource};
use crate::commands::FetchReporter;
use crate::frame_protocol::SharedFrameStore;
use crate::latency::Reporter;
use crate::overlay::SharedOverlayHost;

/// Env var read at startup.
pub const SELFTEST_ENV: &str = "SNAPSHADE_SELFTEST";
/// The SC2 self-exclusion proof mode.
pub const EXCLUSION_MODE: &str = "exclusion";

/// Settle time after setup before the self-test presses the virtual hotkey:
/// the hidden webviews must have loaded `overlay.html` and registered their
/// `frame-ready` listener.
const SETTLE: Duration = Duration::from_millis(1500);
/// Gap between consecutive self-test runs (hide → next press).
const RUN_SPACING: Duration = Duration::from_millis(1000);
/// Upper bound for every display to report `overlay_painted`.
const PAINT_TIMEOUT: Duration = Duration::from_secs(5);
/// Upper bound for every display's page to report its lazy `frame://` fetch (`<N>` mode).
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);

/// Below this mean luminance of the pre-show capture the exclusion test cannot tell a
/// dimmed overlay from the desktop (exit 4, inconclusive).
pub const MIN_DESKTOP_MEAN: f64 = 40.0;
/// Largest allowed |mean(B) − mean(A)| for the excluded capture (8/255).
pub const MAX_EXCLUDED_DELTA: f64 = 8.0;
/// The control capture must be at most this fraction of the pre-show mean (25 % darker).
pub const MAX_CONTROL_RATIO: f64 = 0.75;
/// Pixel stride of the luminance sample — a prime, so periodic UI patterns do not alias.
pub const LUMINANCE_STRIDE: usize = 97;

/// The requested self-test mode, if `SNAPSHADE_SELFTEST` is set and non-empty.
pub fn mode() -> Option<String> {
    std::env::var(SELFTEST_ENV)
        .ok()
        .filter(|value| !value.is_empty())
}

/// Number of end-to-end runs requested by `mode`, if it is a positive integer.
pub fn runs(mode: &str) -> Option<u32> {
    mode.parse::<u32>().ok().filter(|n| *n >= 1)
}

/// `true` for the modes that run the full app (handled by [`schedule`]).
pub fn needs_app(mode: &str) -> bool {
    runs(mode).is_some() || mode == EXCLUSION_MODE
}

/// Run the `preflight` self-test and exit the process. Unknown modes exit 1.
///
/// Called from `setup` BEFORE the hotkey is registered so the probe works even
/// when ⌘⇧9 is owned by another process (another screenshot tool, say).
pub fn run_preflight_or_unknown(mode: &str) -> ! {
    match mode {
        "preflight" => {
            let granted = snapshade_platform_macos::preflight();
            println!("preflight: {}", if granted { "granted" } else { "denied" });
            std::process::exit(if granted { 0 } else { 3 });
        }
        other => {
            eprintln!("unknown {SELFTEST_ENV} mode: {other}");
            std::process::exit(1);
        }
    }
}

/// Median of `values` (mean of the two middle values for an even count).
pub fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[mid]
    } else {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    }
}

/// Dispatch an app-level mode (`needs_app(mode)` is true) at the end of `setup`, after
/// the panels are prepared and the hotkey is registered.
pub fn schedule(app: AppHandle<Wry>, mode: &str) {
    if let Some(runs) = runs(mode) {
        eprintln!("selftest: scheduling {runs} end-to-end run(s)");
        schedule_run(app, runs);
    } else if mode == EXCLUSION_MODE {
        eprintln!("selftest: scheduling the self-exclusion proof (SC2)");
        schedule_exclusion(app);
    } else {
        eprintln!("unknown {SELFTEST_ENV} mode: {mode}");
        std::process::exit(1);
    }
}

/// Install the run reporter the self-test loops wait on.
fn install_reporter(
    app: &AppHandle<Wry>,
) -> mpsc::Receiver<Result<crate::latency::RunReport, String>> {
    let (tx, rx) = mpsc::channel();
    *app.state::<Reporter>()
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = Some(tx);
    rx
}

/// One virtual press and its outcome (completed run report, or the failure reason).
fn press_and_wait(
    app: &AppHandle<Wry>,
    rx: &mpsc::Receiver<Result<crate::latency::RunReport, String>>,
) -> Result<crate::latency::RunReport, String> {
    match trigger(app.clone(), Instant::now(), TriggerSource::SelfTest) {
        Trigger::PermissionMissing => rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap_or_else(|_| Err("permission missing".to_string())),
        Trigger::Started => rx.recv_timeout(PAINT_TIMEOUT).unwrap_or_else(|_| {
            Err(format!(
                "timed out after {} ms waiting for overlay_painted from every display",
                PAINT_TIMEOUT.as_millis()
            ))
        }),
    }
}

/// Install the lazy-fetch reporter (`overlay_frame_fetched` → here) for the `<N>` loop.
fn install_fetch_reporter(app: &AppHandle<Wry>) -> mpsc::Receiver<FetchMessage> {
    let (tx, rx) = mpsc::channel();
    *app.state::<FetchReporter>()
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = Some(tx);
    rx
}

/// `(display_id, gen, bytes, error)` as `commands::overlay_frame_fetched` forwards it.
type FetchMessage = (DisplayId, u64, usize, Option<String>);

/// Lazy-fetch proof for one completed run (Plan 01-10 Task 2): wait for exactly one
/// `overlay_frame_fetched` per expected display within [`FETCH_TIMEOUT`] and check that
/// each reported this generation with no error and `bytes == width × height × 4` (the
/// dimensions from the `FrameStore` meta). `Err` carries the `lazy fetch …` reason.
fn verify_lazy_fetch(
    app: &AppHandle<Wry>,
    fetch_rx: &mpsc::Receiver<FetchMessage>,
    run: &crate::latency::RunReport,
) -> Result<(), String> {
    let expected: BTreeMap<DisplayId, (u32, u32)> = {
        let store = app.state::<SharedFrameStore>();
        let store = store.lock().unwrap_or_else(|p| p.into_inner());
        let topology = app.state::<Arc<CaptureState>>().topology();
        topology
            .displays
            .iter()
            .filter_map(|d| {
                store
                    .get(d.id, run.generation, store.token())
                    .map(|s| (d.id, (s.meta().width, s.meta().height)))
            })
            .collect()
    };
    if expected.len() != run.displays {
        return Err(format!(
            "lazy fetch: store holds {} frame(s) for gen {} but the run painted {}",
            expected.len(),
            run.generation,
            run.displays
        ));
    }
    let deadline = Instant::now() + FETCH_TIMEOUT;
    let mut seen: BTreeMap<DisplayId, ()> = BTreeMap::new();
    while seen.len() < expected.len() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let (display_id, generation, bytes, error) =
            fetch_rx.recv_timeout(remaining).map_err(|_| {
                let missing: Vec<String> = expected
                    .keys()
                    .filter(|id| !seen.contains_key(id))
                    .map(|id| id.to_string())
                    .collect();
                format!(
                    "lazy fetch timed out after {} ms waiting for display(s) {}",
                    FETCH_TIMEOUT.as_millis(),
                    missing.join(", ")
                )
            })?;
        if generation != run.generation {
            // A message from an older generation (never expected; defensive).
            continue;
        }
        let Some((width, height)) = expected.get(&display_id) else {
            return Err(format!(
                "lazy fetch reported unexpected display {display_id}"
            ));
        };
        if let Some(error) = error {
            return Err(format!(
                "lazy fetch failed for display {display_id}: {error}"
            ));
        }
        let want = *width as usize * *height as usize * 4;
        if bytes != want {
            return Err(format!(
                "lazy fetch for display {display_id} returned {bytes} bytes, expected {want} ({width}×{height}×4)"
            ));
        }
        seen.insert(display_id, ());
    }
    Ok(())
}

/// Mode `<N>`: schedule N end-to-end runs on a background thread. Each run must also
/// complete the lazy-fetch proof ([`verify_lazy_fetch`]) before its panels are hidden.
pub fn schedule_run(app: AppHandle<Wry>, runs: u32) {
    let rx = install_reporter(&app);
    let fetch_rx = install_fetch_reporter(&app);

    std::thread::spawn(move || {
        std::thread::sleep(SETTLE);
        let mut totals: Vec<f64> = Vec::with_capacity(runs as usize);
        let mut displays = 0usize;

        for i in 1..=runs {
            if i > 1 {
                std::thread::sleep(RUN_SPACING);
            }
            let outcome = press_and_wait(&app, &rx).and_then(|run| {
                // The page fetches AFTER reporting painted; wait for it BEFORE hiding so the
                // frame is still in the store (hide keeps it; a cancel would drop it).
                verify_lazy_fetch(&app, &fetch_rx, &run).map(|()| run)
            });

            // Restore the desktop before the next run (or before reporting), whatever happened.
            hide_all(&app);
            std::thread::sleep(Duration::from_millis(100));

            match outcome {
                Ok(run) => {
                    totals.push(run.total_ms);
                    displays = run.displays;
                }
                Err(reason) => {
                    println!("selftest failed: run {i}: {reason}");
                    std::process::exit(1);
                }
            }
        }

        // Line changed by Plan 01-10 (as 01-07 documented its own change): `lazy_fetch=ok`
        // says every run's page fetched width × height × 4 bytes over frame:// after paint.
        println!(
            "selftest ok runs={runs} median_total_ms={:.1} displays={displays} lazy_fetch=ok",
            median(&totals)
        );
        std::process::exit(0);
    });
}

/// Mean luminance (`0.2126 R + 0.7152 G + 0.0722 B`, 0–255) of every `stride_px`-th
/// pixel of a top-down RGBA buffer. `NaN` for an empty buffer or a zero stride.
pub fn mean_luminance(rgba: &[u8], stride_px: usize) -> f64 {
    if stride_px == 0 {
        return f64::NAN;
    }
    let mut sum = 0.0_f64;
    let mut n = 0usize;
    for px in rgba.as_chunks::<4>().0.iter().step_by(stride_px) {
        sum += 0.2126 * f64::from(px[0]) + 0.7152 * f64::from(px[1]) + 0.0722 * f64::from(px[2]);
        n += 1;
    }
    if n == 0 {
        f64::NAN
    } else {
        sum / n as f64
    }
}

/// Why one display failed the exclusion proof.
#[derive(Debug, Clone, PartialEq)]
pub enum ExclusionFailure {
    /// The pre-show capture is too dark to tell a dim from the desktop (exit 4).
    Inconclusive { display_id: DisplayId, mean: f64 },
    /// The excluded capture differs from the pre-show capture: overlay chrome leaked (exit 1).
    Leaked { display_id: DisplayId, delta: f64 },
    /// The control capture is not darker: the flag changed nothing (exit 1).
    ControlNotDarker { display_id: DisplayId, ratio: f64 },
}

impl ExclusionFailure {
    /// Process exit code for this failure (0 is never returned here).
    pub fn exit_code(&self) -> i32 {
        match self {
            ExclusionFailure::Inconclusive { .. } => 4,
            ExclusionFailure::Leaked { .. } | ExclusionFailure::ControlNotDarker { .. } => 1,
        }
    }
}

impl std::fmt::Display for ExclusionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExclusionFailure::Inconclusive { display_id, mean } => write!(
                f,
                "selftest exclusion inconclusive: display {display_id} mean {mean:.1} < {MIN_DESKTOP_MEAN:.0} (desktop too dark)"
            ),
            ExclusionFailure::Leaked { display_id, delta } => write!(
                f,
                "selftest exclusion failed: display {display_id}: excluded capture differs from the pre-show capture by {delta:.1} (overlay leaked into the capture)"
            ),
            ExclusionFailure::ControlNotDarker { display_id, ratio } => write!(
                f,
                "selftest exclusion failed: display {display_id}: control capture is not darker (ratio {ratio:.2}) — the exclusion flag has no effect"
            ),
        }
    }
}

/// The per-display verdict over the three means (A = pre-show, B = excluded while
/// visible, C = control while visible). `Ok((delta_excluded, control_ratio))` when the
/// display passes: `delta <= 8.0` and `ratio <= 0.75` (8.0 and 0.75 themselves pass).
pub fn exclusion_verdict(
    display_id: DisplayId,
    mean_a: f64,
    mean_b: f64,
    mean_c: f64,
) -> Result<(f64, f64), ExclusionFailure> {
    // A NaN mean (empty buffer) is never a pass: each check is written as "passes only
    // if the comparison holds", so an incomparable value falls through to the failure.
    let passes = |value: f64, limit: f64| value.partial_cmp(&limit).is_some_and(|o| o.is_le());
    if mean_a.is_nan() || mean_a < MIN_DESKTOP_MEAN {
        return Err(ExclusionFailure::Inconclusive {
            display_id,
            mean: mean_a,
        });
    }
    let delta = (mean_b - mean_a).abs();
    if !passes(delta, MAX_EXCLUDED_DELTA) {
        return Err(ExclusionFailure::Leaked { display_id, delta });
    }
    let ratio = mean_c / mean_a;
    if !passes(ratio, MAX_CONTROL_RATIO) {
        return Err(ExclusionFailure::ControlNotDarker { display_id, ratio });
    }
    Ok((delta, ratio))
}

/// Mode `exclusion`: the SC2 proof described in the module doc.
pub fn schedule_exclusion(app: AppHandle<Wry>) {
    let rx = install_reporter(&app);

    std::thread::spawn(move || {
        std::thread::sleep(SETTLE);
        let fail = |reason: String| -> ! {
            hide_all(&app);
            println!("selftest exclusion failed: {reason}");
            std::process::exit(1);
        };

        // (a) One run: the overlay is now visible with the native frame + dim.
        if let Err(reason) = press_and_wait(&app, &rx) {
            fail(format!("run 1: {reason}"));
        }

        let state = app.state::<Arc<CaptureState>>().inner().clone();
        let topology = state.topology();

        // (b) A = the pre-show capture already in the store (lazy RGBA, converted here).
        let generation;
        let sources = {
            let store = app.state::<SharedFrameStore>();
            let store = store.lock().unwrap_or_else(|p| p.into_inner());
            generation = store.generation();
            topology
                .displays
                .iter()
                .map(|d| (d.id, store.get(d.id, generation, store.token())))
                .collect::<Vec<_>>()
        };
        let mut a: BTreeMap<DisplayId, Arc<CaptureFrame>> = BTreeMap::new();
        for (display_id, source) in sources {
            let Some(source) = source else {
                fail(format!(
                    "display {display_id}: no frame in the store for gen {generation}"
                ));
            };
            match source.rgba() {
                Ok(frame) => {
                    a.insert(display_id, frame);
                }
                Err(reason) => fail(format!("display {display_id}: pre-show rgba: {reason}")),
            }
        }

        // (c) B = excluded capture WITH the overlay visible; (d) C = the control.
        let request = CaptureRequest::interactive();
        let capture =
            |exclusion: SelfExclusion| -> Result<BTreeMap<DisplayId, CaptureFrame>, String> {
                let frames = state
                    .capturer
                    .capture_native(&topology, &request, exclusion)
                    .map_err(|e| format!("{exclusion:?} capture: {e}"))?;
                let mut out = BTreeMap::new();
                for frame in frames {
                    let converted = frame
                        .to_capture_frame()
                        .map_err(|e| format!("{exclusion:?} rgba: {e}"))?;
                    out.insert(converted.display_id, converted);
                }
                Ok(out)
            };
        let b = capture(SelfExclusion::Exclude);
        let c = capture(SelfExclusion::Include);

        // (e) Restore the desktop before judging.
        hide_all(&app);
        std::thread::sleep(Duration::from_millis(100));

        let b = b.unwrap_or_else(|reason| fail(reason));
        let c = c.unwrap_or_else(|reason| fail(reason));

        // (f) Per-display means and verdicts.
        let mut max_delta = 0.0_f64;
        let mut max_ratio = 0.0_f64;
        for (display_id, frame_a) in &a {
            let (Some(frame_b), Some(frame_c)) = (b.get(display_id), c.get(display_id)) else {
                fail(format!(
                    "display {display_id}: missing from the visible captures"
                ));
            };
            let mean_a = mean_luminance(&frame_a.rgba, LUMINANCE_STRIDE);
            let mean_b = mean_luminance(&frame_b.rgba, LUMINANCE_STRIDE);
            let mean_c = mean_luminance(&frame_c.rgba, LUMINANCE_STRIDE);
            eprintln!(
                "exclusion display {display_id}: mean pre-show={mean_a:.1} excluded={mean_b:.1} control={mean_c:.1}"
            );
            match exclusion_verdict(*display_id, mean_a, mean_b, mean_c) {
                Ok((delta, ratio)) => {
                    max_delta = max_delta.max(delta);
                    max_ratio = max_ratio.max(ratio);
                }
                Err(failure) => {
                    println!("{failure}");
                    std::process::exit(failure.exit_code());
                }
            }
        }

        println!(
            "selftest exclusion ok displays={} max_delta_excluded={max_delta:.1} max_control_ratio={max_ratio:.2}",
            a.len()
        );
        std::process::exit(0);
    });
}

fn hide_all(app: &AppHandle<Wry>) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let host = handle.state::<SharedOverlayHost>();
        let mut host = host.lock().unwrap_or_else(|p| p.into_inner());
        if let Err(err) = host.hide_all() {
            eprintln!("selftest: hide_all failed: {err}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{
        exclusion_verdict, mean_luminance, median, needs_app, runs, ExclusionFailure,
        LUMINANCE_STRIDE,
    };

    #[test]
    fn run_counts() {
        assert_eq!(runs("1"), Some(1));
        assert_eq!(runs("20"), Some(20));
        assert_eq!(runs("0"), None);
        assert_eq!(runs("preflight"), None);
        assert!(needs_app("3"));
        assert!(needs_app("exclusion"));
        assert!(!needs_app("preflight"));
    }

    #[test]
    fn median_of_runs() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), 2.5);
        assert!(median(&[]).is_nan());
    }

    #[test]
    fn mean_luminance_over_a_2x2_buffer() {
        // red, green, blue, white — the Rec. 709 weights sum to 1, so the mean is
        // (255·0.2126 + 255·0.7152 + 255·0.0722 + 255) / 4 = 127.5.
        let rgba = [
            255, 0, 0, 255, //
            0, 255, 0, 255, //
            0, 0, 255, 255, //
            255, 255, 255, 255,
        ];
        assert!((mean_luminance(&rgba, 1) - 127.5).abs() < 1e-9);
        // Stride 2 samples the red and blue pixels only.
        let expected = (0.2126 * 255.0 + 0.0722 * 255.0) / 2.0;
        assert!((mean_luminance(&rgba, 2) - expected).abs() < 1e-9);
        assert!(mean_luminance(&[], 1).is_nan());
        assert!(mean_luminance(&rgba, 0).is_nan());
    }

    #[test]
    fn mean_luminance_samples_every_97th_pixel() {
        // 200 black pixels; pixels 0, 97 and 194 are white → the sample is all white.
        let mut rgba = vec![0u8; 200 * 4];
        for i in [0usize, 97, 194] {
            rgba[i * 4..i * 4 + 3].copy_from_slice(&[255, 255, 255]);
        }
        assert!((mean_luminance(&rgba, LUMINANCE_STRIDE) - 255.0).abs() < 1e-9);
        // Pixel 1 white instead of 97 → not sampled.
        let mut rgba = vec![0u8; 200 * 4];
        rgba[4..7].copy_from_slice(&[255, 255, 255]);
        assert_eq!(mean_luminance(&rgba, LUMINANCE_STRIDE), 0.0);
    }

    #[test]
    fn verdict_thresholds() {
        // 8.0 passes, 8.1 fails (excluded delta).
        assert_eq!(exclusion_verdict(1, 100.0, 108.0, 50.0), Ok((8.0, 0.5)));
        assert_eq!(exclusion_verdict(1, 100.0, 92.0, 50.0), Ok((8.0, 0.5)));
        assert!(matches!(
            exclusion_verdict(1, 100.0, 108.1, 50.0),
            Err(ExclusionFailure::Leaked { display_id: 1, .. })
        ));
        // control ratio 0.75 passes, 0.76 fails.
        assert_eq!(exclusion_verdict(2, 100.0, 100.0, 75.0), Ok((0.0, 0.75)));
        assert!(matches!(
            exclusion_verdict(2, 100.0, 100.0, 76.0),
            Err(ExclusionFailure::ControlNotDarker { display_id: 2, .. })
        ));
        // desktop too dark: 39.9 inconclusive, 40 judged.
        let dark = exclusion_verdict(3, 39.9, 39.9, 20.0);
        assert!(matches!(
            dark,
            Err(ExclusionFailure::Inconclusive { display_id: 3, .. })
        ));
        assert_eq!(dark.unwrap_err().exit_code(), 4);
        assert_eq!(exclusion_verdict(3, 40.0, 40.0, 20.0), Ok((0.0, 0.5)));
        // NaN means are never a pass.
        assert!(exclusion_verdict(4, f64::NAN, 100.0, 50.0).is_err());
        assert!(exclusion_verdict(4, 100.0, f64::NAN, 50.0).is_err());
        assert!(exclusion_verdict(4, 100.0, 100.0, f64::NAN).is_err());
        // Exit codes of the hard failures.
        assert_eq!(
            exclusion_verdict(1, 100.0, 120.0, 50.0)
                .unwrap_err()
                .exit_code(),
            1
        );
        assert_eq!(
            exclusion_verdict(1, 100.0, 100.0, 90.0)
                .unwrap_err()
                .exit_code(),
            1
        );
    }
}
