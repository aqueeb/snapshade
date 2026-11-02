//! Zero-copy frozen-frame handoff on Windows (Plan 999.4-01): one WebView2 shared buffer per
//! overlay window, created with `ICoreWebView2Environment12::CreateSharedBuffer` when the
//! window is prepared and posted to its page ONCE per page load
//! (`ICoreWebView2_17::PostSharedBufferToScript`, in reply to `overlay_ready`). The page
//! keeps the `ArrayBuffer`; every capture writes the display's RGBA into the same memory and
//! `frame-ready` (`fmt: "shared"`) tells the page to paint from it. A 33 MB 4K frame no
//! longer crosses the custom-protocol transport (0.35–0.55 s per display in 999.3), and the
//! page's annotation preview and export read the same lossless bytes, so there is no second
//! fetch either.
//!
//! Access: the buffer is posted only to the overlay window's own top-level document (never
//! to a frame), so the T-01-28 property holds — pixels reach exactly the page that paints
//! them. The page gets READ_WRITE access: WebView2 documents that a write to a READ_ONLY
//! buffer is an access violation in the renderer, and the page writing its own frame harms
//! nothing because Rust never reads the buffer back.
//!
//! Threading: every COM call happens on the main thread — `WebviewWindow::with_webview`
//! runs its closure synchronously there (tauri-runtime-wry short-circuits
//! `send_user_message` on the main thread), which is how [`SharedFrameBuffer::create`]
//! returns a result. The raw pointer is written from the capture worker threads: plain
//! shared memory, not a COM call. A capture racing a display-change `reprepare` can drop
//! the last `Arc` off the main thread; the final `Release` then runs on the worker, which
//! WebView2's WRL objects tolerate.

use std::ptr;
use std::sync::{Arc, Mutex};

use tauri::webview::PlatformWebview;
use tauri::{WebviewWindow, Wry};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2Environment12, ICoreWebView2SharedBuffer, ICoreWebView2_17,
    COREWEBVIEW2_SHARED_BUFFER_ACCESS_READ_WRITE,
};
use windows_core::{Interface, PCWSTR};

/// Env switch for A/B measurement on one binary: `fetch` creates no shared buffers, so
/// every capture takes the `frame://` path (`fmt: "fetch"`, the 999.3 behaviour).
pub const HANDOFF_ENV: &str = "SNAPSHADE_FRAME_HANDOFF";

/// `true` when [`HANDOFF_ENV`] asks for the fetch path.
pub fn shared_handoff_disabled() -> bool {
    std::env::var(HANDOFF_ENV)
        .map(|v| v.trim().eq_ignore_ascii_case("fetch"))
        .unwrap_or(false)
}

/// One display's shared frame memory: `width * height * 4` bytes owned by WebView2, mapped
/// into this process at `ptr` and, once posted, into the page as an `ArrayBuffer`.
pub struct SharedFrameBuffer {
    buffer: ICoreWebView2SharedBuffer,
    ptr: *mut u8,
    len: usize,
    width: u32,
    height: u32,
}

// SAFETY: see the module doc — COM calls stay on the main thread; `ptr` is a view of shared
// memory that lives as long as `buffer`, which lives as long as the last `Arc<Self>`.
unsafe impl Send for SharedFrameBuffer {}
unsafe impl Sync for SharedFrameBuffer {}

impl std::fmt::Debug for SharedFrameBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedFrameBuffer")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("len", &self.len)
            .finish()
    }
}

impl SharedFrameBuffer {
    /// Create a `width * height * 4`-byte buffer in `window`'s WebView2 environment. Main
    /// thread only (the closure must run synchronously to hand the result back). `Err`
    /// means this window stays on the fetch path: a runtime without
    /// `ICoreWebView2Environment12`, an allocation failure, or a call off the main thread.
    pub fn create(
        window: &WebviewWindow<Wry>,
        width: u32,
        height: u32,
    ) -> Result<Arc<Self>, String> {
        let len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4))
            .filter(|n| *n > 0)
            .ok_or_else(|| format!("invalid frame size {width}x{height}"))?;
        let slot: Arc<Mutex<Option<Result<SharedFrameBuffer, String>>>> = Arc::default();
        let out = Arc::clone(&slot);
        window
            .with_webview(move |webview| {
                let result = create_in(&webview, width, height, len);
                *out.lock().unwrap_or_else(|p| p.into_inner()) = Some(result);
            })
            .map_err(|e| format!("with_webview: {e}"))?;
        let result = slot.lock().unwrap_or_else(|p| p.into_inner()).take();
        match result {
            Some(Ok(buffer)) => Ok(Arc::new(buffer)),
            Some(Err(reason)) => Err(reason),
            None => Err("with_webview did not run synchronously (not the main thread)".into()),
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Byte length (`width * height * 4`).
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Copy `rgba` (exactly `len` bytes) into the shared memory. Any thread. `false` when the
    /// length does not match — the frame is not this display's size, nothing is written.
    pub fn write(&self, rgba: &[u8]) -> bool {
        if rgba.len() != self.len {
            return false;
        }
        // SAFETY: `ptr` addresses `len` writable bytes for as long as `self` lives (the COM
        // buffer is only closed in `Drop`), and `rgba` is process memory of our own that
        // cannot overlap the mapping.
        unsafe { ptr::copy_nonoverlapping(rgba.as_ptr(), self.ptr, self.len) };
        true
    }

    /// Post the buffer to `window`'s current document as an `ArrayBuffer`;
    /// `additional_data_json` becomes `event.additionalData`. Main thread. The COM outcome
    /// is logged from the closure; the returned error is a dispatch failure only.
    pub fn post(
        self: &Arc<Self>,
        window: &WebviewWindow<Wry>,
        additional_data_json: String,
    ) -> Result<(), String> {
        let me = Arc::clone(self);
        let label = window.label().to_string();
        window
            .with_webview(move |webview| {
                if let Err(reason) = post_in(&webview, &me.buffer, &additional_data_json) {
                    eprintln!("{label}: shared frame buffer post failed: {reason}");
                }
            })
            .map_err(|e| format!("with_webview: {e}"))
    }
}

impl Drop for SharedFrameBuffer {
    fn drop(&mut self) {
        // SAFETY: closing releases the mapping; a page still holding the ArrayBuffer sees
        // it detached, which the page's length check treats as "no buffer".
        unsafe {
            let _ = self.buffer.Close();
        }
    }
}

fn create_in(
    webview: &PlatformWebview,
    width: u32,
    height: u32,
    len: usize,
) -> Result<SharedFrameBuffer, String> {
    let environment = webview
        .environment()
        .cast::<ICoreWebView2Environment12>()
        .map_err(|e| {
            format!("WebView2 runtime has no shared buffers (ICoreWebView2Environment12): {e}")
        })?;
    // SAFETY: COM calls on the UI thread against a live environment; the out-pointer is valid.
    let (buffer, ptr) = unsafe {
        let buffer = environment
            .CreateSharedBuffer(len as u64)
            .map_err(|e| format!("CreateSharedBuffer({len}): {e}"))?;
        let mut ptr: *mut u8 = ptr::null_mut();
        buffer
            .Buffer(&mut ptr)
            .map_err(|e| format!("SharedBuffer.Buffer: {e}"))?;
        (buffer, ptr)
    };
    if ptr.is_null() {
        return Err("shared buffer mapped to a null pointer".to_string());
    }
    Ok(SharedFrameBuffer {
        buffer,
        ptr,
        len,
        width,
        height,
    })
}

fn post_in(
    webview: &PlatformWebview,
    buffer: &ICoreWebView2SharedBuffer,
    json: &str,
) -> Result<(), String> {
    // SAFETY: UI thread; the controller and its CoreWebView2 live as long as the window.
    unsafe {
        let core = webview
            .controller()
            .CoreWebView2()
            .map_err(|e| format!("CoreWebView2: {e}"))?;
        let core = core.cast::<ICoreWebView2_17>().map_err(|e| {
            format!("WebView2 runtime has no PostSharedBufferToScript (ICoreWebView2_17): {e}")
        })?;
        let wide: Vec<u16> = json.encode_utf16().chain(std::iter::once(0)).collect();
        core.PostSharedBufferToScript(
            buffer,
            COREWEBVIEW2_SHARED_BUFFER_ACCESS_READ_WRITE,
            PCWSTR(wide.as_ptr()),
        )
        .map_err(|e| format!("PostSharedBufferToScript: {e}"))
    }
}
