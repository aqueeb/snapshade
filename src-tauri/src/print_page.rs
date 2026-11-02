//! Printing through a hidden webview page (EXPORT-05 on Windows, where there is no
//! `NSPrintOperation`): the flattened PNG is held in managed state and served to the
//! `print` window over the `print://` scheme (`http://print.localhost/image.png` on
//! WebView2); the page calls `window.print()` once the image has loaded, which opens the
//! system print dialog with the image scaled to the page by the print stylesheet.
//!
//! The window is created ONCE at startup ([`prepare`]) and only reloaded + shown per job:
//! `export_image` is a synchronous command on the main thread, and building a
//! `WebviewWindow` inside it wedged WebView2 for the rest of the process (no export line,
//! every later overlay black) in the 2026-10-31 Windows hands-on run — PITFALLS Pitfall 11.

use std::sync::Mutex;

use snapshade_core::capture::CaptureFrame;
use snapshade_core::encode::{encode, ImageFormat};
use snapshade_platform_api::{PlatformError, Printer};
use tauri::{http, AppHandle, Manager, Runtime, WebviewUrl, WebviewWindowBuilder, Wry};

pub const PRINT_LABEL: &str = "print";

/// The PNG the print page shows (one job at a time).
#[derive(Default)]
pub struct PrintJob(pub Mutex<Option<Vec<u8>>>);

/// `Printer` that opens the hidden print page. Used on Windows; available everywhere.
pub struct WebviewPrinter {
    app: AppHandle<Wry>,
}

impl WebviewPrinter {
    pub fn new(app: AppHandle<Wry>) -> Self {
        Self { app }
    }
}

/// Create the hidden print window (call from `setup`, never from a command). The page
/// loads with no job (its image 404s, nothing prints) and is reloaded per job.
pub fn prepare(app: &AppHandle<Wry>) -> Result<(), PlatformError> {
    if app.get_webview_window(PRINT_LABEL).is_some() {
        return Ok(());
    }
    WebviewWindowBuilder::new(app, PRINT_LABEL, WebviewUrl::App("print.html".into()))
        .title("Print")
        .inner_size(640.0, 480.0)
        .visible(false)
        // Never take focus at startup: a hidden window with the default focused=true still
        // became the foreground window for a moment on Windows.
        .focused(false)
        .build()
        .map(|_| ())
        .map_err(|e| PlatformError::Os(format!("print window: {e}")))
}

impl Printer for WebviewPrinter {
    fn print_image(&self, frame: &CaptureFrame) -> Result<(), PlatformError> {
        let png = encode(&frame.rgba, frame.width, frame.height, ImageFormat::Png)
            .map_err(|e| PlatformError::Os(format!("png encode: {e}")))?;
        *self
            .app
            .state::<PrintJob>()
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(png);
        // Never build the window here (synchronous command on the main thread — see the
        // module doc); `prepare` made it at startup. Reload so the page picks up the job.
        let window = self.app.get_webview_window(PRINT_LABEL).ok_or_else(|| {
            PlatformError::Os("print window was not prepared at startup".to_string())
        })?;
        window
            .eval("window.location.reload()")
            .map_err(|e| PlatformError::Os(format!("print window reload: {e}")))?;
        window
            .show()
            .map_err(|e| PlatformError::Os(format!("print window show: {e}")))?;
        let _ = window.set_focus();
        Ok(())
    }
}

fn not_found() -> http::Response<Vec<u8>> {
    http::Response::builder()
        .status(404)
        .body(Vec::new())
        .expect("static 404 response")
}

/// Register `print://localhost/image.png` → the current job's PNG.
pub fn register<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder.register_uri_scheme_protocol("print", |ctx, request| {
        let path = request.uri().path();
        if path != "/image.png" {
            return not_found();
        }
        let job = ctx.app_handle().state::<PrintJob>();
        let png = job.0.lock().unwrap_or_else(|p| p.into_inner()).clone();
        match png {
            Some(bytes) => http::Response::builder()
                .status(200)
                .header("Content-Type", "image/png")
                .header("Access-Control-Allow-Origin", "*")
                .body(bytes)
                .unwrap_or_else(|_| not_found()),
            None => not_found(),
        }
    })
}
