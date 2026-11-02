//! Explorer reveal and URL opening.

use std::path::Path;
use std::process::Command;

use snapshade_platform_api::PlatformError;
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

pub fn reveal_file(path: &Path) -> Result<(), PlatformError> {
    Command::new("explorer.exe")
        .arg(format!("/select,{}", path.display()))
        .spawn()
        .map(|_| ())
        .map_err(|e| PlatformError::Os(format!("explorer /select: {e}")))
}

pub fn open_url(url: &str) -> Result<(), PlatformError> {
    let target = HSTRING::from(url);
    // SAFETY: documented ShellExecuteW call; the strings outlive the call.
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(target.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecuteW returns a pseudo-HINSTANCE > 32 on success.
    if (result.0 as isize) > 32 {
        Ok(())
    } else {
        Err(PlatformError::Os(format!(
            "ShellExecuteW failed ({})",
            result.0 as isize
        )))
    }
}
