//! Export paths (EXPORT-01..06): one flattened RGBA crop from the overlay page → clipboard
//! (PNG + TIFF, written BEFORE the page dismisses the overlay), instant save (⌘S: last
//! folder, else default folder, else the OS default, collision-safe name, no dialog), Save
//! As (native dialog with PNG/JPG/BMP filters), print (native dialog scaled to fit), each
//! followed by a notification when enabled. All platform calls go through the
//! `snapshade-platform-api` traits; this file never names AppKit.

use std::path::{Path, PathBuf};

use serde::Serialize;
use snapshade_core::capture::CaptureFrame;
use snapshade_core::encode::{encode_with, ImageFormat};
use snapshade_core::filename::{screenshot_stem, unique_path_on_disk, LocalTime};
use snapshade_core::settings::Settings;
use snapshade_platform_api::{ImageClipboard, NotificationAction, Notifier, Printer, Shell};
use tauri::{AppHandle, Manager, Wry};

use crate::settings_store;
use crate::strings;

/// The platform exporters, resolved once at startup (managed state).
pub struct Exporters {
    pub clipboard: Box<dyn ImageClipboard + Send + Sync>,
    pub printer: Box<dyn Printer + Send + Sync>,
    pub shell: Box<dyn Shell + Send + Sync>,
    pub notifier: Box<dyn Notifier + Send + Sync>,
}

/// Stderr-only notifier (dev binaries that are not a bundle; platforms without a path yet).
pub struct LogNotifier;

impl Notifier for LogNotifier {
    fn notify(
        &self,
        title: &str,
        body: &str,
        action: NotificationAction,
    ) -> Result<(), snapshade_platform_api::PlatformError> {
        eprintln!("notification: {title} — {body} ({action:?})");
        Ok(())
    }
}

/// Every exporter answers `Unsupported` (Windows until Phase 5 lands the adapters).
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
struct UnsupportedExporter;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
impl ImageClipboard for UnsupportedExporter {
    fn write_image(
        &self,
        _frame: &CaptureFrame,
    ) -> Result<(), snapshade_platform_api::PlatformError> {
        Err(snapshade_platform_api::PlatformError::Unsupported)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
impl Printer for UnsupportedExporter {
    fn print_image(
        &self,
        _frame: &CaptureFrame,
    ) -> Result<(), snapshade_platform_api::PlatformError> {
        Err(snapshade_platform_api::PlatformError::Unsupported)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
impl Shell for UnsupportedExporter {
    fn reveal_file(&self, _path: &Path) -> Result<(), snapshade_platform_api::PlatformError> {
        Err(snapshade_platform_api::PlatformError::Unsupported)
    }
    fn open_url(&self, _url: &str) -> Result<(), snapshade_platform_api::PlatformError> {
        Err(snapshade_platform_api::PlatformError::Unsupported)
    }
}

impl Exporters {
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    pub fn unsupported() -> Self {
        Self {
            clipboard: Box::new(UnsupportedExporter),
            printer: Box::new(UnsupportedExporter),
            shell: Box::new(UnsupportedExporter),
            notifier: Box::new(LogNotifier),
        }
    }
}

/// What the page asked for (`x-kind` header of `export_image`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportKind {
    Copy,
    Save,
    SaveAs,
    Print,
}

impl ExportKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "copy" | "clipboard" => Some(Self::Copy),
            "save" => Some(Self::Save),
            "saveAs" | "save_as" => Some(Self::SaveAs),
            "print" => Some(Self::Print),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOutcome {
    pub kind: String,
    /// The written file for save / save-as; `None` when cancelled or not a file export.
    pub path: Option<String>,
    /// `true` when the user cancelled a dialog.
    pub cancelled: bool,
}

fn now_local() -> LocalTime {
    use chrono::{Datelike, Timelike};
    let now = chrono::Local::now();
    LocalTime {
        month: now.month(),
        day: now.day(),
        hour: now.hour(),
        minute: now.minute(),
        second: now.second(),
    }
}

/// The OS default save folder when the user has not chosen one: Desktop (macOS screenshot
/// convention) or `Documents/Snapshade` on Windows, falling back to the home directory.
pub fn os_default_folder(app: &AppHandle<Wry>) -> PathBuf {
    use tauri::path::BaseDirectory;
    let path = app.path();
    #[cfg(target_os = "macos")]
    let preferred = path.resolve("", BaseDirectory::Desktop);
    #[cfg(not(target_os = "macos"))]
    let preferred = path.resolve("Snapshade", BaseDirectory::Document);
    preferred
        .or_else(|_| path.resolve("", BaseDirectory::Home))
        .unwrap_or_else(|_| std::env::temp_dir())
}

/// Encode `frame` as `format` (honouring the PNG optimisation switch) and write it.
pub fn write_file(
    frame: &CaptureFrame,
    path: &Path,
    format: ImageFormat,
    optimize_png: bool,
) -> Result<(), String> {
    let bytes = encode_with(&frame.rgba, frame.width, frame.height, format, optimize_png)
        .map_err(|e| format!("encode: {e}"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("write {}: {e}", path.display()))
}

/// The format of `settings` with the live JPEG quality folded in.
pub fn default_format(settings: &Settings) -> ImageFormat {
    match settings.default_format {
        ImageFormat::Jpeg { .. } => ImageFormat::Jpeg {
            quality: settings.jpeg_quality,
        },
        other => other,
    }
}

fn notify(app: &AppHandle<Wry>, settings: &Settings, body: &str, action: NotificationAction) {
    if !settings.show_notifications {
        return;
    }
    let exporters = app.state::<Exporters>();
    if let Err(err) = exporters.notifier.notify(strings::APP_NAME, body, action) {
        eprintln!("notify: {err}");
    }
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// EXPORT-01/02: write PNG + TIFF now; the page dismisses only after this returns.
pub fn copy(app: &AppHandle<Wry>, frame: &CaptureFrame) -> Result<ExportOutcome, String> {
    let settings = settings_store::current(app);
    app.state::<Exporters>()
        .clipboard
        .write_image(frame)
        .map_err(|e| e.to_string())?;
    notify(
        app,
        &settings,
        strings::NOTIFY_COPIED,
        NotificationAction::None,
    );
    Ok(ExportOutcome {
        kind: "copy".into(),
        path: None,
        cancelled: false,
    })
}

/// EXPORT-03: ⌘S — no dialog, last-used (else default, else OS) folder, unique name.
pub fn save_instant(app: &AppHandle<Wry>, frame: &CaptureFrame) -> Result<ExportOutcome, String> {
    let settings = settings_store::current(app);
    let folder = settings.save_folder(os_default_folder(app));
    let format = default_format(&settings);
    let path = unique_path_on_disk(&folder, &screenshot_stem(now_local()), format.extension());
    write_file(frame, &path, format, settings.optimize_png)?;
    notify(
        app,
        &settings,
        &strings::notify_saved(&file_label(&path)),
        NotificationAction::RevealFile(path.clone()),
    );
    Ok(ExportOutcome {
        kind: "save".into(),
        path: Some(path.display().to_string()),
        cancelled: false,
    })
}

/// EXPORT-04: the native Save As with PNG / JPG / BMP choices. Main thread (modal).
pub fn save_as(app: &AppHandle<Wry>, frame: &CaptureFrame) -> Result<ExportOutcome, String> {
    let settings = settings_store::current(app);
    let folder = settings.save_folder(os_default_folder(app));
    let format = default_format(&settings);
    let stem = screenshot_stem(now_local());
    #[cfg(target_os = "macos")]
    let _ = snapshade_platform_macos::activate_app();
    let mut dialog = rfd::FileDialog::new()
        .set_title(strings::SAVE_DIALOG_TITLE)
        .set_directory(&folder)
        .set_file_name(format!("{stem}.{}", format.extension()))
        .set_can_create_directories(true);
    // The default format's filter first so the dialog pre-selects it.
    let filters: [(&str, &[&str]); 3] = [
        ("PNG", &["png"]),
        ("JPEG", &["jpg", "jpeg"]),
        ("BMP", &["bmp"]),
    ];
    let first = match format {
        ImageFormat::Png => 0,
        ImageFormat::Jpeg { .. } => 1,
        ImageFormat::Bmp => 2,
    };
    for i in 0..3 {
        let (name, exts) = filters[(first + i) % 3];
        dialog = dialog.add_filter(name, exts);
    }
    let Some(mut path) = dialog.save_file() else {
        return Ok(ExportOutcome {
            kind: "saveAs".into(),
            path: None,
            cancelled: true,
        });
    };
    let chosen = path
        .extension()
        .and_then(|e| e.to_str())
        .and_then(|e| ImageFormat::from_extension(e, settings.jpeg_quality));
    let chosen = match chosen {
        Some(f) => f,
        None => {
            path.set_extension(format.extension());
            format
        }
    };
    write_file(frame, &path, chosen, settings.optimize_png)?;
    if let Some(dir) = path.parent() {
        let dir = dir.to_path_buf();
        settings_store::update(app, |s| s.last_save_folder = Some(dir));
    }
    notify(
        app,
        &settings,
        &strings::notify_saved(&file_label(&path)),
        NotificationAction::RevealFile(path.clone()),
    );
    Ok(ExportOutcome {
        kind: "saveAs".into(),
        path: Some(path.display().to_string()),
        cancelled: false,
    })
}

/// EXPORT-05: native print dialog, image scaled to fit. Main thread (modal).
pub fn print(app: &AppHandle<Wry>, frame: &CaptureFrame) -> Result<ExportOutcome, String> {
    #[cfg(target_os = "macos")]
    let _ = snapshade_platform_macos::activate_app();
    app.state::<Exporters>()
        .printer
        .print_image(frame)
        .map_err(|e| e.to_string())?;
    Ok(ExportOutcome {
        kind: "print".into(),
        path: None,
        cancelled: false,
    })
}

/// HOTKEY-04: save every display of `frames` to the default folder with no overlay.
pub fn save_fullscreen(
    app: &AppHandle<Wry>,
    frames: &[CaptureFrame],
) -> Result<Vec<PathBuf>, String> {
    let settings = settings_store::current(app);
    let folder = settings.save_folder(os_default_folder(app));
    let format = default_format(&settings);
    let stem = screenshot_stem(now_local());
    let mut paths = Vec::with_capacity(frames.len());
    for (i, frame) in frames.iter().enumerate() {
        let stem = if frames.len() > 1 {
            format!("{stem} (display {})", i + 1)
        } else {
            stem.clone()
        };
        let path = unique_path_on_disk(&folder, &stem, format.extension());
        write_file(frame, &path, format, settings.optimize_png)?;
        paths.push(path);
    }
    if let Some(first) = paths.first() {
        notify(
            app,
            &settings,
            &strings::notify_saved(&file_label(first)),
            NotificationAction::RevealFile(first.clone()),
        );
    }
    Ok(paths)
}

/// Handle a notification click (runs on the main thread).
pub fn on_notification_click(app: &AppHandle<Wry>, action: NotificationAction) {
    match action {
        NotificationAction::None => {}
        NotificationAction::RevealFile(path) => {
            if let Err(err) = app.state::<Exporters>().shell.reveal_file(&path) {
                eprintln!("reveal {}: {err}", path.display());
            }
        }
        NotificationAction::OpenOptions => {
            if let Err(err) = crate::aux_windows::show(app, crate::aux_windows::OPTIONS) {
                eprintln!("options: {err}");
            }
        }
        NotificationAction::OpenUrl(url) => {
            if let Err(err) = app.state::<Exporters>().shell.open_url(&url) {
                eprintln!("open {url}: {err}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ExportKind;

    #[test]
    fn export_kind_parses_the_page_vocabulary() {
        assert_eq!(ExportKind::parse("copy"), Some(ExportKind::Copy));
        assert_eq!(ExportKind::parse("clipboard"), Some(ExportKind::Copy));
        assert_eq!(ExportKind::parse("save"), Some(ExportKind::Save));
        assert_eq!(ExportKind::parse("saveAs"), Some(ExportKind::SaveAs));
        assert_eq!(ExportKind::parse("print"), Some(ExportKind::Print));
        assert_eq!(ExportKind::parse("upload"), None);
    }
}
