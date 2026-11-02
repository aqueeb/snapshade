//! `WinOverlayHost`: one hidden, borderless, always-on-top `WebviewWindow` per monitor on
//! Windows (Phase 5 port of `overlay.rs`). There is no native frame layer: the page paints
//! the frozen frame onto `#frame` before it reports painted. Since Plan 999.4-01 the pixels
//! reach the page through a WebView2 shared buffer owned by this host (one per window, see
//! `shared_frame_win.rs`; `FrameReady.fmt == "shared"`); a window without one — runtime
//! without the interfaces, `SNAPSHADE_FRAME_HANDOFF=fetch` — keeps the `frame://` fetch
//! path (`fmt == "fetch"`). Windows are positioned from the topology's logical rects (tao's
//! monitor positions are reliable on Windows) and take the foreground when revealed so the
//! keyboard reaches them (PITFALLS Pitfall 8).
//!
//! Since Plan 999.4-03 the windows are **never hidden**: hiding and re-showing a WebView2
//! window cost 45–55 ms per press and stalled 100–650 ms one press in five (the renderer
//! resuming). Each window is shown once at `prepare`, layered + click-through (tao's
//! ignore-cursor-events flag) at alpha 0, so it is invisible and inert; a capture emits
//! `frame-ready` to the page and the window turns opaque + clickable only when that page
//! reports painted ([`reveal_after_paint`]) — so the user never sees a stale frame and there
//! is no show cost at all. Dismissing ([`WinOverlayHost::conceal_all`]) flips it back and
//! returns the foreground to the window that had it. `background_color` stays black (a
//! renderer restart would otherwise be a white rectangle).

use std::sync::{Arc, Mutex, TryLockError};

use snapshade_core::geometry::DisplayId;
use snapshade_core::topology::DisplayTopology;
use snapshade_platform_api::{OverlayHost, PlatformError};
use snapshade_platform_windows::window_fx;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Wry};

use crate::frame_protocol::SharedFrameStore;
use crate::payloads::SharedBufferInfo;
use crate::shared_frame_win::{shared_handoff_disabled, SharedFrameBuffer, HANDOFF_ENV};

pub type SharedOverlayHost = Arc<Mutex<WinOverlayHost>>;

struct Overlay {
    id: DisplayId,
    label: String,
    window: WebviewWindow<Wry>,
    /// The monitor's rect in physical px `(x, y, w, h)` — what the window must cover.
    physical: (i32, i32, u32, u32),
    scale: f64,
    /// The zero-copy frame handoff (Plan 999.4-01), sized `w * h * 4` for this monitor;
    /// `None` keeps this window on the fetch path.
    shared: Option<Arc<SharedFrameBuffer>>,
    /// Raw `HWND` for the alpha / tool-window flips (Plan 999.4-03).
    hwnd: isize,
    /// Opaque and clickable (a capture is on screen) vs transparent and click-through.
    revealed: bool,
}

impl Overlay {
    /// Invisible + click-through: alpha 0 and `WS_EX_TRANSPARENT`. The window stays layered
    /// (set once at prepare through tao's ignore-cursor-events flag) — toggling the layered
    /// style itself rebuilds the DWM surface and shows black until WebView2 re-presents.
    /// Idempotent.
    fn conceal(&mut self) {
        window_fx::set_alpha(self.hwnd, 0);
        window_fx::set_click_through(self.hwnd, true);
        self.revealed = false;
    }

    /// Opaque + clickable: the two attributes flipped back, nothing rebuilt. Idempotent.
    fn reveal(&mut self) {
        if window_fx::ensure_shown_no_activate(self.hwnd) {
            eprintln!("{}: was hidden or minimised; shown again", self.label);
        }
        window_fx::set_click_through(self.hwnd, false);
        window_fx::set_alpha(self.hwnd, 255);
        self.revealed = true;
    }
}

/// Create the window's shared frame buffer, or explain once why it stays on the fetch path.
fn make_shared(
    window: &WebviewWindow<Wry>,
    label: &str,
    width: u32,
    height: u32,
) -> Option<Arc<SharedFrameBuffer>> {
    if shared_handoff_disabled() {
        eprintln!("{label}: shared frame buffer disabled ({HANDOFF_ENV}=fetch); using frame://");
        return None;
    }
    match SharedFrameBuffer::create(window, width, height) {
        Ok(buffer) => {
            eprintln!(
                "{label}: shared frame buffer {width}x{height} ({:.1} MB)",
                buffer.len() as f64 / (1024.0 * 1024.0)
            );
            Some(buffer)
        }
        Err(reason) => {
            eprintln!("{label}: shared frame buffer unavailable, using frame:// ({reason})");
            None
        }
    }
}

/// The physical rect of `d` (its logical rect was derived as `physical / scale`, so this
/// round-trips exactly for whole-pixel monitor origins).
fn physical_rect(d: &snapshade_core::geometry::DisplayInfo) -> (i32, i32, u32, u32) {
    (
        (d.logical.x * d.scale).round() as i32,
        (d.logical.y * d.scale).round() as i32,
        d.physical.w,
        d.physical.h,
    )
}

pub struct WinOverlayHost {
    app: AppHandle<Wry>,
    overlays: Vec<Overlay>,
    epoch: u64,
    /// The foreground window before the overlay took it (recorded by `show_all`, handed
    /// back by `conceal_all`). 0 = none.
    prev_foreground: isize,
}

impl WinOverlayHost {
    pub fn new(app: AppHandle<Wry>) -> Self {
        Self {
            app,
            overlays: Vec::new(),
            epoch: 0,
            prev_foreground: 0,
        }
    }

    /// `true` while any overlay window is opaque (a capture is on screen).
    pub fn is_revealed(&self) -> bool {
        self.overlays.iter().any(|o| o.revealed)
    }

    /// Make `display_id`'s window opaque and clickable; the first window revealed for a
    /// capture also takes the keyboard. `Ok(false)` when it already was.
    pub fn reveal(&mut self, display_id: DisplayId) -> Result<bool, String> {
        let first = !self.is_revealed();
        let Some(overlay) = self.overlays.iter_mut().find(|o| o.id == display_id) else {
            return Err(format!("no overlay window for display {display_id}"));
        };
        if overlay.revealed {
            return Ok(false);
        }
        overlay.reveal();
        if first {
            // The hotkey press grants foreground rights; the keyboard goes to this window.
            // tao's `set_focus` (its forced-activation routine) first, a plain
            // SetForegroundWindow second; the outcome is logged because a silently missing
            // focus means Esc and the shortcuts go to the wrong app.
            let _ = overlay.window.set_focus();
            let hwnd = overlay.hwnd;
            let label = overlay.label.clone();
            let taken = window_fx::foreground_window() == hwnd || window_fx::take_foreground(hwnd);
            if !taken {
                eprintln!("{label}: foreground NOT taken on reveal");
            }
            // Diagnostic: an activation that is reverted shortly after the reveal means the
            // keyboard silently goes elsewhere; say so, with the window that got it.
            std::thread::spawn(move || {
                for (delay_ms, next) in [(150u64, 450u64), (600, 0)] {
                    std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                    let fg = window_fx::foreground_window();
                    if fg != hwnd {
                        eprintln!(
                            "{label}: foreground left the overlay within {} ms (now {fg:#x})",
                            delay_ms + if next == 0 { 150 } else { 0 }
                        );
                        return;
                    }
                    if next == 0 {
                        eprintln!("{label}: foreground held for 750 ms");
                    }
                }
            });
        }
        Ok(true)
    }

    /// Dismiss: every window transparent + click-through again, the foreground handed back.
    pub fn conceal_all(&mut self) {
        let any = self.is_revealed();
        for o in &mut self.overlays {
            if o.revealed {
                o.conceal();
            }
        }
        if any {
            let ours = self.overlays.iter().any(|o| o.hwnd == self.prev_foreground);
            if !ours && !window_fx::restore_foreground(self.prev_foreground) {
                eprintln!("overlay: previous foreground window could not be restored");
            }
        }
        self.prev_foreground = 0;
    }

    pub fn display_ids(&self) -> Vec<DisplayId> {
        self.overlays.iter().map(|o| o.id).collect()
    }

    pub fn labels(&self) -> Vec<(DisplayId, String)> {
        self.overlays
            .iter()
            .map(|o| (o.id, o.label.clone()))
            .collect()
    }

    /// Every overlay window sits exactly on its monitor's physical rect. Checked after
    /// `show_all`; a mismatch is logged and lands in the latency row as `placement_ok:
    /// false` (the mixed-DPI case PITFALLS Pitfall 4 warns about).
    pub fn placement_ok(&self) -> bool {
        let mut ok = true;
        for o in &self.overlays {
            let pos = o.window.outer_position().map(|p| (p.x, p.y));
            let size = o.window.inner_size().map(|s| (s.width, s.height));
            match (pos, size) {
                (Ok((x, y)), Ok((w, h))) if (x, y, w, h) == o.physical => {}
                (pos, size) => {
                    ok = false;
                    eprintln!(
                        "overlay {}: placed at {pos:?} {size:?}, expected {:?}",
                        o.label, o.physical
                    );
                }
            }
        }
        ok
    }

    /// The shared frame buffers by display — cloned `Arc`s, so the capture thread writes
    /// after this lock is released. A display without one takes the fetch path.
    pub fn shared_buffers(&self) -> Vec<(DisplayId, Arc<SharedFrameBuffer>)> {
        self.overlays
            .iter()
            .filter_map(|o| o.shared.as_ref().map(|b| (o.id, Arc::clone(b))))
            .collect()
    }

    /// `overlay_ready`: post `display_id`'s buffer to its page (once per page load; a
    /// reloaded page gets the same buffer again). Main thread. `Ok(false)` when the
    /// display has no buffer (fetch path).
    pub fn post_shared_buffer(&self, display_id: DisplayId) -> Result<bool, String> {
        let Some(overlay) = self.overlays.iter().find(|o| o.id == display_id) else {
            return Err(format!("no overlay window for display {display_id}"));
        };
        let Some(shared) = &overlay.shared else {
            return Ok(false);
        };
        let info = SharedBufferInfo {
            display_id,
            width: shared.width(),
            height: shared.height(),
            scale: overlay.scale,
            epoch: self.epoch,
        };
        shared.post(&overlay.window, info.to_json())?;
        Ok(true)
    }

    /// Display change: close every window, bump the label epoch, prepare afresh.
    pub fn reprepare(&mut self, topo: &DisplayTopology) -> Result<usize, PlatformError> {
        let old = std::mem::take(&mut self.overlays);
        let replaced = old.len();
        self.epoch += 1;
        for overlay in old {
            if let Err(err) = overlay.window.close() {
                eprintln!("overlay {}: close: {err}", overlay.label);
            }
        }
        self.prepare(topo)?;
        Ok(replaced)
    }
}

impl OverlayHost for WinOverlayHost {
    fn prepare(&mut self, topo: &DisplayTopology) -> Result<(), PlatformError> {
        for d in &topo.displays {
            let physical = physical_rect(d);
            // A display that already has a window keeps it as is: the window, its shared
            // buffer and the page's view of that buffer are sized for one monitor, so every
            // topology change goes through `reprepare` (close all, build afresh) — there is
            // no in-place resize, and `prepare` only ever runs with an empty list.
            if self.overlays.iter().any(|o| o.id == d.id) {
                continue;
            }
            let label = crate::overlay_label(d.id, self.epoch);
            let window = WebviewWindowBuilder::new(
                &self.app,
                &label,
                WebviewUrl::App("overlay.html".into()),
            )
            .title("Snapshade overlay")
            .visible(false)
            .decorations(false)
            .shadow(false)
            .resizable(false)
            .focused(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .background_color(tauri::window::Color(0, 0, 0, 255))
            .position(d.logical.x, d.logical.y)
            .inner_size(d.logical.w, d.logical.h)
            .build()
            .map_err(|e| PlatformError::Os(format!("{label}: build: {e}")))?;
            let shared = make_shared(&window, &label, physical.2, physical.3);
            let hwnd = window
                .hwnd()
                .map(|h| h.0 as isize)
                .map_err(|e| PlatformError::Os(format!("{label}: hwnd: {e}")))?;
            let mut overlay = Overlay {
                id: d.id,
                label,
                window,
                physical,
                scale: d.scale,
                shared,
                hwnd,
                revealed: false,
            };
            // Plan 999.4-03: shown once, for good — layered (tao's flag, kept for the window's
            // lifetime), invisible and click-through BEFORE the show so nothing ever flashes,
            // and without taking the foreground.
            overlay
                .window
                .set_ignore_cursor_events(true)
                .map_err(|e| PlatformError::Os(format!("{}: layered: {e}", overlay.label)))?;
            overlay.conceal();
            let foreground = window_fx::foreground_window();
            overlay
                .window
                .show()
                .map_err(|e| PlatformError::Os(format!("{}: show: {e}", overlay.label)))?;
            // tao rewrote the extended style on show (layered + transparent survive, the
            // tool-window bit does not); re-assert what is ours.
            overlay.conceal();
            window_fx::add_tool_window_style(hwnd);
            if window_fx::foreground_window() == hwnd {
                let _ = window_fx::restore_foreground(foreground);
            }
            eprintln!(
                "{}: shown transparent (layered={})",
                overlay.label,
                window_fx::is_layered(hwnd)
            );
            self.overlays.push(overlay);
        }
        if self.overlays.is_empty() {
            return Err(PlatformError::Os(
                "no overlay could be prepared".to_string(),
            ));
        }
        Ok(())
    }

    /// Nothing to show (the windows are always shown): arm the capture — remember who has
    /// the foreground so `conceal_all` can give it back. The windows turn opaque one by one
    /// in [`reveal_after_paint`].
    fn show_all(&mut self) -> Result<(), PlatformError> {
        if self.overlays.is_empty() {
            return Err(PlatformError::Os("no overlay window prepared".to_string()));
        }
        if !self.is_revealed() {
            self.prev_foreground = window_fx::foreground_window();
        }
        Ok(())
    }

    fn hide_all(&mut self) -> Result<(), PlatformError> {
        self.conceal_all();
        for o in &self.overlays {
            if let Err(err) = self.app.emit_to(o.label.as_str(), "overlay-hidden", ()) {
                eprintln!("overlay-hidden emit to {} failed: {err}", o.label);
            }
        }
        Ok(())
    }
}

/// What [`reveal_after_paint`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevealOutcome {
    /// The window was made opaque by this call (the caller stamps `t_reveal`).
    Revealed,
    /// Nothing to do: the generation is no longer live (Esc came first), or the window
    /// already was opaque.
    Skipped,
    /// The host lock is held — by this same thread, inside a display-change resync — so
    /// the caller must run its whole paint handling again from a later event-loop turn.
    Busy,
}

/// `overlay_painted` (main thread): turn `display_id`'s window opaque now that its page has
/// painted generation `gen` — only if that generation is still the live one (a late paint
/// report after Esc must not resurrect a dismissed overlay).
///
/// The host lock is taken with `try_lock`: a command can run inside WebView2's nested message
/// pump while a display-change resync holds the lock on this same thread (the 999.4-01
/// deadlock). A busy lock is reported as [`RevealOutcome::Busy`] and the caller
/// (`commands::record_paint`) re-dispatches the whole paint handling through the event
/// loop, so the later attempt re-checks liveness and stamps the reveal with the paint
/// (Plan 999.4-04; the first version revealed from a detached closure that did neither).
pub fn reveal_after_paint(app: &AppHandle<Wry>, display_id: DisplayId, gen: u64) -> RevealOutcome {
    let live = {
        let store = app.state::<SharedFrameStore>();
        let store = store.lock().unwrap_or_else(|p| p.into_inner());
        store.generation() == gen && !store.is_empty()
    };
    if !live {
        return RevealOutcome::Skipped;
    }
    let host: SharedOverlayHost = Arc::clone(&app.state::<SharedOverlayHost>());
    let attempt = host.try_lock();
    let outcome = match attempt {
        Ok(mut guard) => match guard.reveal(display_id) {
            Ok(true) => RevealOutcome::Revealed,
            Ok(false) => RevealOutcome::Skipped,
            Err(err) => {
                eprintln!("reveal display {display_id}: {err}");
                RevealOutcome::Skipped
            }
        },
        Err(TryLockError::Poisoned(poisoned)) => {
            let mut guard = poisoned.into_inner();
            match guard.reveal(display_id) {
                Ok(true) => RevealOutcome::Revealed,
                _ => RevealOutcome::Skipped,
            }
        }
        Err(TryLockError::WouldBlock) => RevealOutcome::Busy,
    };
    outcome
}
