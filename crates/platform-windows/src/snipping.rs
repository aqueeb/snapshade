//! PKG-05: on Windows 11 the PrintScreen key may be owned by Snipping Tool
//! (`HKCU\Control Panel\Keyboard\PrintScreenKeyForSnippingEnabled` = 1), in which case
//! `RegisterHotKey(VK_SNAPSHOT)` either fails or never fires. The app checks this and
//! shows an explanation with a link to the owning setting.

use windows::core::w;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

/// The Settings page that owns the key (Accessibility → Keyboard → "Use the Print screen
/// key to open screen capture").
pub const SNIPPING_TOOL_SETTINGS_URL: &str = "ms-settings:easeofaccess-keyboard";

/// `Some(true)` when Snipping Tool owns PrintScreen, `Some(false)` when the value is 0,
/// `None` when the value does not exist (older Windows).
pub fn print_screen_owned_by_snipping_tool() -> Option<bool> {
    let mut value: u32 = 0;
    let mut size: u32 = std::mem::size_of::<u32>() as u32;
    // SAFETY: valid out-pointers sized for a REG_DWORD.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Control Panel\\Keyboard"),
            w!("PrintScreenKeyForSnippingEnabled"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    if status.is_ok() {
        Some(value != 0)
    } else {
        None
    }
}
