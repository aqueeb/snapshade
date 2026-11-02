//! DXGI desktop duplication (`IDXGIOutputDuplication`) — the primary capture backend since
//! Plan 999.4-02. The desktop compositor's current image of a monitor is copied GPU-side into
//! a staging texture and read back, instead of GDI re-rendering every window into a DIB
//! (`BitBlt | CAPTUREBLT`, ~50 ms per 4K monitor). Chosen over the WinRT
//! `Windows.Graphics.Capture` API for a one-shot screenshot: no capture session to start per
//! press, no border or indicator, and nothing beyond `windows` crate features already in the
//! tree.
//!
//! Lifecycle: one [`OutputState`] per monitor — a D3D11 device on the output's adapter, the
//! duplication, and a staging texture that always holds the LAST acquired desktop image.
//! States are created only by [`Duplicator::prime`] (startup, and after a display change —
//! background thread): the hotkey path never creates a device (the first D3D11 device in a
//! process took ~1.6 s on the test box) and never waits for a monitor's lock; a monitor
//! that is not primed yet takes the GDI path for that press (Plan 999.4-04). Windows allows
//! one duplication per output per process, so a state is created under its monitor's slot
//! lock, never in two threads at once. On capture, `AcquireNextFrame(0)`:
//! * a frame → copy it into the staging texture, release it;
//! * `DXGI_ERROR_WAIT_TIMEOUT` → nothing was presented since the last acquire, so the staging
//!   texture already is the current desktop image (nothing to do);
//! * `DXGI_ERROR_ACCESS_LOST` (mode change, secure desktop, driver reset) → the state is
//!   dropped and recreated once.
//!
//! Then the staging texture is mapped and converted BGRA → RGBA. Anything the backend cannot
//! do (rotated output, a format other than B8G8R8A8, no frame yet, an adapter without
//! duplication support — remote sessions, some virtual displays) is an `Err` the caller
//! answers with the GDI path. The cursor is never in the desktop image; the caller composites
//! it when asked.
//!
//! Measured 2026-10-31 (two 4K monitors, release): 10–27 ms per monitor against 70–100 ms
//! for GDI, pixel-identical output. Two things worth knowing: a fresh duplication yields no
//! frame until the compositor presents (0.5–1.9 s on an idle desktop — hence `prime` on a
//! background thread at startup), and DXGI turns a DPI-UNAWARE process system-DPI-aware on
//! first use (the app is Per-Monitor-V2 by manifest, so only unaware test binaries notice).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, TryLockError};
use std::time::{Duration, Instant};

use windows::core::Interface;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_FLAG, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_MODE_ROTATION_IDENTITY, DXGI_MODE_ROTATION_UNSPECIFIED,
    DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput, IDXGIOutput1,
    IDXGIOutputDuplication, IDXGIResource, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT,
    DXGI_OUTDUPL_FRAME_INFO,
};
use windows::Win32::System::Performance::QueryPerformanceCounter;

use crate::pixels::bgra_rows_to_rgba;

/// Env switch for A/B measurement on one binary: `gdi` skips this backend entirely.
pub const CAPTURE_ENV: &str = "SNAPSHADE_CAPTURE";

/// `true` when [`CAPTURE_ENV`] asks for the GDI path.
pub fn disabled_by_env() -> bool {
    std::env::var(CAPTURE_ENV)
        .map(|v| v.trim().eq_ignore_ascii_case("gdi"))
        .unwrap_or(false)
}

/// Where the time went in one grab (µs), for the per-capture log line.
#[derive(Debug, Clone, Copy, Default)]
pub struct GrabTiming {
    pub acquire_us: u128,
    pub copy_us: u128,
    pub readback_us: u128,
    /// `true` when the staging texture was reused because nothing had been presented.
    pub reused: bool,
}

struct OutputState {
    _device: ID3D11Device,
    context: ID3D11DeviceContext,
    duplication: IDXGIOutputDuplication,
    staging: ID3D11Texture2D,
    width: u32,
    height: u32,
    /// `true` once the staging texture holds a desktop image.
    primed: bool,
    /// After an invalidation: the QPC time the next acquired frame must have been presented
    /// after (`DXGI_OUTDUPL_FRAME_INFO::LastPresentTime`). 0 = any frame.
    min_present_qpc: i64,
}

// SAFETY: D3D11 devices and DXGI duplications are free-threaded COM objects; every call on a
// state goes through its own Mutex, so the immediate context is never used concurrently.
unsafe impl Send for OutputState {}

enum GrabError {
    /// Duplication lost (mode change, secure desktop); recreate and retry once.
    AccessLost,
    /// No frame has been acquired yet and none arrived within the timeout.
    NotYet,
    /// Anything else — the caller falls back to GDI.
    Other(String),
}

impl OutputState {
    /// Create the device, duplication and staging texture for the output whose
    /// `DXGI_OUTPUT_DESC.Monitor` is `hmonitor`; `expected` is the monitor's pixel size.
    fn create(hmonitor: isize, expected: (u32, u32)) -> Result<Self, String> {
        let (adapter, output) = find_output(hmonitor)?;
        // SAFETY: plain DXGI / D3D11 object creation; every out-pointer is a valid local.
        unsafe {
            let desc = output.GetDesc().map_err(|e| format!("GetDesc: {e}"))?;
            if desc.Rotation != DXGI_MODE_ROTATION_IDENTITY
                && desc.Rotation != DXGI_MODE_ROTATION_UNSPECIFIED
            {
                return Err(format!("rotated output ({:?})", desc.Rotation));
            }
            let mut device: Option<ID3D11Device> = None;
            let mut context: Option<ID3D11DeviceContext> = None;
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_FLAG(0),
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .map_err(|e| format!("D3D11CreateDevice: {e}"))?;
            let device = device.ok_or("D3D11CreateDevice returned no device")?;
            let context = context.ok_or("D3D11CreateDevice returned no context")?;
            let output1: IDXGIOutput1 = output.cast().map_err(|e| format!("IDXGIOutput1: {e}"))?;
            let duplication = output1
                .DuplicateOutput(&device)
                .map_err(|e| format!("DuplicateOutput: {e}"))?;
            let dd = duplication.GetDesc();
            if dd.ModeDesc.Format != DXGI_FORMAT_B8G8R8A8_UNORM {
                return Err(format!(
                    "desktop format {:?} is not B8G8R8A8",
                    dd.ModeDesc.Format
                ));
            }
            if (dd.ModeDesc.Width, dd.ModeDesc.Height) != expected {
                return Err(format!(
                    "duplication is {}x{}, monitor is {}x{}",
                    dd.ModeDesc.Width, dd.ModeDesc.Height, expected.0, expected.1
                ));
            }
            let staging_desc = D3D11_TEXTURE2D_DESC {
                Width: expected.0,
                Height: expected.1,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };
            let mut staging: Option<ID3D11Texture2D> = None;
            device
                .CreateTexture2D(&staging_desc, None, Some(&mut staging))
                .map_err(|e| format!("CreateTexture2D(staging): {e}"))?;
            let staging = staging.ok_or("CreateTexture2D returned no texture")?;
            Ok(Self {
                _device: device,
                context,
                duplication,
                staging,
                width: expected.0,
                height: expected.1,
                primed: false,
                min_present_qpc: 0,
            })
        }
    }

    /// Acquire the newest desktop image (or keep the cached one on `WAIT_TIMEOUT`), then
    /// read the staging texture back as RGBA into `dst`. After
    /// [`Duplicator::invalidate_all`] only a frame presented after the invalidation counts
    /// (`LastPresentTime`, a QPC stamp, against `min_present_qpc`): an older one — or a
    /// pointer-only update, stamp 0 — is released and the wait goes on until `timeout_ms`
    /// is spent.
    fn grab(&mut self, timeout_ms: u32, dst: &mut [u8]) -> Result<GrabTiming, GrabError> {
        let mut timing = GrabTiming::default();
        let t0 = Instant::now();
        let deadline = t0 + Duration::from_millis(u64::from(timeout_ms));
        loop {
            let remaining = deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
                .min(u128::from(u32::MAX)) as u32;
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource: Option<IDXGIResource> = None;
            // SAFETY: valid out-pointers; the frame is released before this function returns.
            let acquired = unsafe {
                self.duplication
                    .AcquireNextFrame(remaining, &mut info, &mut resource)
            };
            match acquired {
                Ok(()) => {
                    let t1 = Instant::now();
                    let texture: ID3D11Texture2D = match resource.as_ref().map(|r| r.cast()) {
                        Some(Ok(t)) => t,
                        Some(Err(e)) => {
                            // SAFETY: a frame was acquired.
                            let _ = unsafe { self.duplication.ReleaseFrame() };
                            return Err(GrabError::Other(format!("frame is not a texture: {e}")));
                        }
                        None => {
                            let _ = unsafe { self.duplication.ReleaseFrame() };
                            return Err(GrabError::Other(
                                "AcquireNextFrame gave no resource".into(),
                            ));
                        }
                    };
                    if info.LastPresentTime < self.min_present_qpc {
                        // Composed before the invalidation (it still shows what was just
                        // concealed), or no desktop update at all: wait for a newer one.
                        // SAFETY: a frame was acquired.
                        let _ = unsafe { self.duplication.ReleaseFrame() };
                        if Instant::now() >= deadline {
                            return Err(GrabError::NotYet);
                        }
                        continue;
                    }
                    // SAFETY: both textures belong to this state's device; same size and format.
                    unsafe {
                        self.context.CopyResource(&self.staging, &texture);
                        let _ = self.duplication.ReleaseFrame();
                    }
                    self.primed = true;
                    self.min_present_qpc = 0;
                    timing.copy_us = t1.elapsed().as_micros();
                }
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => {
                    if !self.primed {
                        return Err(GrabError::NotYet);
                    }
                    timing.reused = true;
                }
                Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => return Err(GrabError::AccessLost),
                Err(e) => return Err(GrabError::Other(format!("AcquireNextFrame: {e}"))),
            }
            break;
        }
        timing.acquire_us = t0.elapsed().as_micros() - timing.copy_us;

        let t2 = Instant::now();
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: the staging texture is CPU-readable; it is unmapped below on every path.
        unsafe {
            self.context
                .Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .map_err(|e| GrabError::Other(format!("Map(staging): {e}")))?;
        }
        let pitch = mapped.RowPitch as usize;
        let (width, height) = (self.width as usize, self.height as usize);
        if mapped.pData.is_null() || pitch < width * 4 {
            // SAFETY: mapped above.
            unsafe { self.context.Unmap(&self.staging, 0) };
            return Err(GrabError::Other(format!(
                "mapped staging texture is unusable (pitch {pitch}, width {width})"
            )));
        }
        // SAFETY: the mapping covers `pitch * height` bytes until Unmap.
        let src = unsafe { std::slice::from_raw_parts(mapped.pData as *const u8, pitch * height) };
        bgra_rows_to_rgba(src, pitch, width, height, dst);
        // SAFETY: mapped above.
        unsafe { self.context.Unmap(&self.staging, 0) };
        timing.readback_us = t2.elapsed().as_micros();
        Ok(timing)
    }
}

/// The adapter and output whose `DXGI_OUTPUT_DESC.Monitor` is `hmonitor`.
fn find_output(hmonitor: isize) -> Result<(IDXGIAdapter1, IDXGIOutput), String> {
    // SAFETY: factory enumeration; every call returns an owned COM object or an error.
    unsafe {
        let factory: IDXGIFactory1 =
            CreateDXGIFactory1().map_err(|e| format!("CreateDXGIFactory1: {e}"))?;
        let mut adapter_index = 0;
        while let Ok(adapter) = factory.EnumAdapters1(adapter_index) {
            adapter_index += 1;
            let mut output_index = 0;
            while let Ok(output) = adapter.EnumOutputs(output_index) {
                output_index += 1;
                if let Ok(desc) = output.GetDesc() {
                    if desc.Monitor.0 as isize == hmonitor {
                        return Ok((adapter, output));
                    }
                }
            }
        }
    }
    Err("no DXGI output for this monitor".to_string())
}

/// One monitor's slot in the [`Duplicator`] map: its size (so a lookup never needs the
/// inner lock) and the duplication state, `None` until created or after it was dropped on
/// an error. Windows allows ONE duplication of an output per process (`DuplicateOutput`
/// fails with `E_INVALIDARG` for a second one — and was seen to block for seconds first), so
/// creation happens under the slot's lock, never in two threads at once; the map lock is
/// held only to look a slot up or insert an empty one, so one monitor never waits for
/// another's device creation or acquire.
struct Slot {
    size: (u32, u32),
    state: Mutex<Option<OutputState>>,
}

/// All per-monitor duplication slots, keyed by `HMONITOR`. Shared by the capture threads
/// (one monitor per thread; each slot has its own lock).
#[derive(Default)]
pub struct Duplicator {
    outputs: Mutex<HashMap<isize, Arc<Slot>>>,
}

impl std::fmt::Debug for Duplicator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self
            .outputs
            .lock()
            .map(|m| m.len())
            .unwrap_or_else(|p| p.into_inner().len());
        f.debug_struct("Duplicator").field("outputs", &n).finish()
    }
}

/// How long the first acquire of a fresh duplication may wait for the compositor's initial
/// full frame (hotkey path; `prime` retries longer off the hotkey path).
const FIRST_FRAME_TIMEOUT_MS: u32 = 100;

impl Duplicator {
    /// The slot for `hmonitor` at `expected` pixels — the existing one, or a fresh empty one
    /// (also when the size changed: the old slot's state is dropped once its last user is
    /// done). Holds the map lock only for the lookup.
    fn slot_for(&self, hmonitor: isize, expected: (u32, u32)) -> Arc<Slot> {
        let mut map = self.outputs.lock().unwrap_or_else(|p| p.into_inner());
        match map.get(&hmonitor) {
            Some(slot) if slot.size == expected => Arc::clone(slot),
            _ => {
                let slot = Arc::new(Slot {
                    size: expected,
                    state: Mutex::new(None),
                });
                map.insert(hmonitor, Arc::clone(&slot));
                slot
            }
        }
    }

    /// Create the state for `hmonitor` and acquire its first frame (startup, background
    /// thread). A fresh duplication delivers nothing until the compositor presents — on an
    /// idle desktop that can take seconds (measured 0.7–1.9 s), and a mouse move counts — so
    /// this retries for up to ~5 s. The slot's lock is taken per attempt (one creation or one
    /// 100 ms acquire at most), never across the loop: a press meanwhile finds it free or
    /// busy for an instant and takes the GDI path ([`Duplicator::capture`] never waits).
    /// Returns what stopped it, if anything.
    pub fn prime(&self, hmonitor: isize, expected: (u32, u32)) -> Result<GrabTiming, String> {
        if disabled_by_env() {
            return Err(format!("{CAPTURE_ENV}=gdi"));
        }
        let slot = self.slot_for(hmonitor, expected);
        let mut scratch = vec![0u8; expected.0 as usize * expected.1 as usize * 4];
        let mut last = String::new();
        for _ in 0..50 {
            let mut guard = slot.state.lock().unwrap_or_else(|p| p.into_inner());
            if guard.is_none() {
                *guard = Some(OutputState::create(hmonitor, expected)?);
            }
            let state = guard.as_mut().expect("state was just created");
            if state.primed {
                // A capture got there first (and acquired a frame): nothing left to prime.
                return Ok(GrabTiming {
                    reused: true,
                    ..GrabTiming::default()
                });
            }
            match state.grab(FIRST_FRAME_TIMEOUT_MS, &mut scratch) {
                Ok(timing) => return Ok(timing),
                Err(GrabError::NotYet) => last = "no frame within the priming window".to_string(),
                Err(GrabError::AccessLost) => {
                    *guard = None;
                    return Err("access lost while priming".to_string());
                }
                Err(GrabError::Other(reason)) => {
                    *guard = None;
                    return Err(reason);
                }
            }
        }
        Err(last)
    }

    /// Forget the cached desktop images: the next grab must acquire a frame PRESENTED AFTER
    /// this call (`LastPresentTime` later than now; it waits up to
    /// [`FIRST_FRAME_TIMEOUT_MS`]) or report `Err` so the caller uses GDI. Used right after
    /// the overlay was concealed for a press-while-up capture, when the cached image — and a
    /// frame the compositor produced before the conceal — still shows the overlay. (A fixed
    /// sleep and `DwmFlush` were both tried: the first is a guess, the second waits for the
    /// NEXT present, which on an idle desktop was 1.1 s away.)
    pub fn invalidate_all(&self) {
        let mut now = 0i64;
        // SAFETY: valid out-pointer.
        let _ = unsafe { QueryPerformanceCounter(&mut now) };
        let slots: Vec<Arc<Slot>> = self
            .outputs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .cloned()
            .collect();
        for slot in slots {
            let mut guard = slot.state.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(state) = guard.as_mut() {
                state.primed = false;
                state.min_present_qpc = now;
            }
        }
    }

    /// Read the current desktop image of `hmonitor` (`expected` pixels) into `dst` as RGBA.
    /// `Err(reason)` means "use GDI for this monitor" — including when the slot is busy
    /// (priming holds it for one creation or one 100 ms acquire) or was never primed: the
    /// hotkey path never waits for a lock and never creates a device, except to recreate
    /// one after `ACCESS_LOST` (a mode change; the driver is loaded by then).
    pub fn capture(
        &self,
        hmonitor: isize,
        expected: (u32, u32),
        dst: &mut [u8],
    ) -> Result<GrabTiming, String> {
        if disabled_by_env() {
            return Err(format!("{CAPTURE_ENV}=gdi"));
        }
        let slot = self.slot_for(hmonitor, expected);
        let mut guard = match slot.state.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => return Err("duplication busy (priming)".to_string()),
        };
        let mut recreated = false;
        loop {
            if guard.is_none() {
                if !recreated {
                    // Never primed, or dropped after an error: the hotkey path does not pay
                    // a device creation (the first D3D11 device in a process took ~1.6 s on
                    // the test box) — `prime` does, off the hotkey path.
                    return Err("not primed yet".to_string());
                }
                *guard = Some(OutputState::create(hmonitor, expected)?);
            }
            let state = guard.as_mut().expect("state is present");
            // A state that has not delivered a frame yet (priming in progress, or just
            // recreated) may wait for the compositor's first full frame; a primed one must
            // not wait at all.
            let timeout = if state.primed {
                0
            } else {
                FIRST_FRAME_TIMEOUT_MS
            };
            match state.grab(timeout, dst) {
                Ok(timing) => return Ok(timing),
                Err(GrabError::NotYet) => return Err("no desktop frame yet".to_string()),
                Err(GrabError::AccessLost) if !recreated => {
                    // Dropped before the recreation: the output may only be duplicated once.
                    *guard = None;
                    recreated = true;
                }
                Err(GrabError::AccessLost) => {
                    *guard = None;
                    return Err("duplication access lost twice".to_string());
                }
                Err(GrabError::Other(reason)) => {
                    *guard = None;
                    return Err(reason);
                }
            }
        }
    }
}
