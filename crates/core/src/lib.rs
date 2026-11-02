//! Snapshade pure core.
//!
//! Everything in this crate is platform-independent and compiles on every host.
//! It must never depend on `tauri`, `objc2`, `screencapturekit`, `core-graphics`,
//! `xcap`, or `windows` (D-06, D-18); CI enforces this with `cargo tree` and the
//! `windows-latest` `cargo check --workspace` job.
//!
//! Phase 1 Plan 01-01 pins the *type contracts* only. Behaviour (coordinate
//! conversion, hit-testing, encoding, hotkey parsing) arrives in Plan 01-04.

pub mod capture;
pub mod encode;
pub mod filename;
pub mod geometry;
pub mod hotkey;
pub mod settings;
pub mod topology;
