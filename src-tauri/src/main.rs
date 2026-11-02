// Prevents an additional console window on Windows in release builds; no effect on macOS.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    snapshade_lib::run()
}
