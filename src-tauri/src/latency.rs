//! D-09 latency instrumentation for one capture run, written as the RESEARCH Q6
//! JSON Lines schema (Plan 01-07 Task 1; timestamp semantics re-defined for Plan B by
//! Plan 01-09 and documented here by Plan 01-11).
//!
//! Clocks: `Instant` for intra-process deltas (authoritative), epoch microseconds
//! for correlating with the webview's `performance.timeOrigin`. Points:
//!
//! * `t0`     hotkey handler entry
//! * `t1`     every per-display capture joined (`capture_native`: retained `CGImage`s, no
//!   RGBA copy)
//! * `t2[id]` that display's `CGImage` was installed on its panel's native layer
//!   (`FrameLayer::set_contents`), stamped by the controller right after each layer set.
//!   Since Plan 01-09 this is NOT "served over `frame://`": the RGBA conversion behind
//!   `frame://` is lazy and off the critical path, and its `X-Frame-Served-Us` header is
//!   informational only
//! * `t_show` the last `show_and_make_key()` returned (main thread) — the frame is on
//!   screen from here
//! * `t4[id]` Rust receipt of `overlay_painted` from that display's page (its double rAF
//!   after show; conservative: includes the IPC hop); the JS wall stamp is logged next to it
//!
//! `total_ms` is the max `t4` over displays — a conservative upper bound that includes the
//! page's post-show rAF term, not the moment the frame became visible (`t_show`); the gate
//! record states which figure it gates. `t_show_ms` is `Option<f64>`: `None` (JSON `null`,
//! `notes: "t_show missing"`) when `record_shown` never ran, which `scripts/spike-stats.ts`
//! reports as a violation instead of coercing to 0 (REVIEW WR-08). `fmt` is always `native`
//! from Plan 01-09 on (the frame reaches the screen as layer contents, never as encoded
//! bytes). Schema stays 1 — the field set is unchanged.
//!
//! Once every expected display has painted,
//! [`Timings::row`] builds one [`LatencyRow`] and [`append_row`] appends it to
//! `~/Library/Logs/Snapshade/spike-latency.jsonl` and mirrors it to stderr as
//! `latency {json}`. The row carries timings, display geometry, identifiers and the
//! scenario tag only — never pixel buffers, window titles, or typed text (RESEARCH
//! Security Domain; T-01-18).
//!
//! Scenario tagging (D-10) needs no relaunch: `SNAPSHADE_SCENARIO` wins, else the
//! trimmed contents of `~/Library/Logs/Snapshade/scenario.txt` re-read on every press,
//! else `untagged`. Self-test runs are tagged `selftest` by the trigger source, not by
//! the tag file (T-01-19), so `scripts/spike-stats.ts` can exclude them.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use snapshade_core::geometry::DisplayId;
use snapshade_core::topology::DisplayTopology;

/// `schema` field of every row (RESEARCH Q6 line 469).
pub const SCHEMA: u32 = 1;
/// Env var that overrides the scenario tag.
pub const SCENARIO_ENV: &str = "SNAPSHADE_SCENARIO";
/// File name of the JSONL log inside [`log_dir`].
pub const LOG_FILE: &str = "spike-latency.jsonl";
/// File name of the human-editable scenario tag inside [`log_dir`].
pub const SCENARIO_FILE: &str = "scenario.txt";
/// Tag used when neither the env var nor the file provides one.
pub const UNTAGGED: &str = "untagged";
/// Tag of every row produced by the self-test loop.
pub const SELFTEST_SCENARIO: &str = "selftest";

/// Managed state: `Arc<Mutex<Timings>>`.
pub type SharedTimings = Arc<Mutex<Timings>>;

/// What a completed run reports to an in-process observer (the self-test).
#[derive(Debug, Clone, PartialEq)]
pub struct RunReport {
    pub generation: u64,
    pub total_ms: f64,
    pub displays: usize,
    pub placement_ok: Option<bool>,
}

/// Where a run's outcome is delivered besides the log: `Ok` on completion, `Err`
/// with the `aborted:` / `permission missing` reason otherwise. Installed only by
/// the self-test; `None` in normal operation.
pub type Reporter = Arc<Mutex<Option<Sender<Result<RunReport, String>>>>>;

/// One display as logged: geometry only (RESEARCH Q6 `displays[]`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DisplayRow {
    pub id: DisplayId,
    pub scale: f64,
    /// Physical bitmap size `[w, h]` in physical px.
    pub px: [u32; 2],
    /// Logical size `[w, h]` in points.
    pub pt: [f64; 2],
    /// Logical origin `[x, y]` in the global top-left, y-down space.
    pub origin_pt: [f64; 2],
}

impl DisplayRow {
    /// The topology snapshot in log form, in topology order (primary first).
    pub fn from_topology(topo: &DisplayTopology) -> Vec<DisplayRow> {
        topo.displays
            .iter()
            .map(|d| DisplayRow {
                id: d.id,
                scale: d.scale,
                px: [d.physical.w, d.physical.h],
                pt: [d.logical.w, d.logical.h],
                origin_pt: [d.logical.x, d.logical.y],
            })
            .collect()
    }
}

/// One JSON Lines row — the exact RESEARCH Q6 schema plus `selftest` and `fmt`
/// (Plan 01-07). Contains no byte buffers by construction (T-01-18).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LatencyRow {
    pub schema: u32,
    pub run: u64,
    pub scenario: String,
    /// RFC 3339 UTC, second resolution.
    pub ts: String,
    pub hw: String,
    pub os: String,
    /// `"battery"` | `"ac"` | `"unknown"`.
    pub power: String,
    pub displays: Vec<DisplayRow>,
    pub t0_epoch_us: u64,
    pub t1_ms: f64,
    pub t2_ms: BTreeMap<String, f64>,
    /// `None` (JSON `null`) when `record_shown` never ran; `notes` then says
    /// `t_show missing` and the stats script treats the row as a violation.
    pub t_show_ms: Option<f64>,
    pub t4_ms: BTreeMap<String, f64>,
    pub t4_js_wall_ms: BTreeMap<String, f64>,
    /// Windows (Plan 999.4-03): the last overlay window turned opaque — the moment the user
    /// sees the frame, stamped after that display's paint report. `None` on macOS, where the
    /// panel is visible from `t_show` on. Included in `total_ms`.
    pub t_reveal_ms: Option<f64>,
    pub total_ms: f64,
    pub placement_ok: bool,
    pub selftest: bool,
    pub fmt: String,
    pub notes: String,
}

/// Per-run context known at trigger time (everything in the row that is not a timing).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RunContext {
    pub scenario: String,
    pub selftest: bool,
    pub fmt: String,
    pub displays: Vec<DisplayRow>,
}

/// Timestamps for the current generation.
#[derive(Debug, Default)]
pub struct Timings {
    generation: u64,
    t0: Option<Instant>,
    t0_epoch_us: u128,
    t1_ms: f64,
    t2_ms: BTreeMap<DisplayId, f64>,
    t_show_ms: Option<f64>,
    t4_ms: BTreeMap<DisplayId, f64>,
    t4_js_wall_ms: BTreeMap<DisplayId, f64>,
    t_reveal_ms: BTreeMap<DisplayId, f64>,
    expected: HashSet<DisplayId>,
    placement_ok: Option<bool>,
    context: RunContext,
    completed: bool,
}

impl Timings {
    /// Start a new generation (called by the controller right after `t1`).
    pub fn begin(
        &mut self,
        generation: u64,
        t0: Instant,
        t0_epoch_us: u128,
        t1_ms: f64,
        displays: impl IntoIterator<Item = DisplayId>,
        context: RunContext,
    ) {
        *self = Self {
            generation,
            t0: Some(t0),
            t0_epoch_us,
            t1_ms,
            expected: displays.into_iter().collect(),
            context,
            ..Self::default()
        };
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// `t2[id]`: the display's frame is on its panel's layer (Plan B; called by the
    /// controller right after each `FrameLayer::set_contents`). Ignored for
    /// another generation.
    pub fn record_served(&mut self, generation: u64, display_id: DisplayId, served_us: u128) {
        if generation != self.generation || self.t0.is_none() {
            return;
        }
        let delta_us = served_us.saturating_sub(self.t0_epoch_us);
        // First stamp wins: on macOS the controller stamps "frame on the layer" before
        // the page's post-paint fetch calls the frame:// handler; on Windows (`fmt:
        // "fetch"`) the handler's stamp is the first and means "frame served to the page".
        self.t2_ms
            .entry(display_id)
            .or_insert(delta_us as f64 / 1000.0);
    }

    /// How many of `generation`'s displays have reported painted so far (0 for
    /// another generation) — the Windows watchdog's "nothing painted" test.
    pub fn painted_count(&self, generation: u64) -> usize {
        if generation != self.generation {
            return 0;
        }
        self.t4_ms.len()
    }

    /// Stamp `t_show` (right after the last `show_and_make_key`) and the
    /// placement check result.
    pub fn record_shown(&mut self, generation: u64, placement_ok: bool) {
        if generation != self.generation {
            return;
        }
        if let Some(t0) = self.t0 {
            self.t_show_ms = Some(ms_since(t0));
        }
        self.placement_ok = Some(placement_ok);
    }

    /// Windows: the display's overlay window was made opaque after its paint report
    /// (Plan 999.4-03). Call BEFORE `record_painted` for the same display so the completing
    /// row includes it. Ignored for another generation.
    pub fn record_revealed(&mut self, generation: u64, display_id: DisplayId) {
        if generation != self.generation {
            return;
        }
        if let Some(t0) = self.t0 {
            self.t_reveal_ms.insert(display_id, ms_since(t0));
        }
    }

    /// Rust receipt of `overlay_painted`. Returns the run report once every
    /// expected display has painted (exactly once per generation).
    pub fn record_painted(
        &mut self,
        generation: u64,
        display_id: DisplayId,
        t_wall_ms: f64,
    ) -> Option<RunReport> {
        if generation != self.generation || !self.expected.contains(&display_id) {
            return None;
        }
        let t0 = self.t0?;
        self.t4_ms.insert(display_id, ms_since(t0));
        self.t4_js_wall_ms
            .insert(display_id, t_wall_ms - self.t0_epoch_us as f64 / 1000.0);
        if self.completed || self.t4_ms.len() < self.expected.len() {
            return None;
        }
        self.completed = true;
        Some(RunReport {
            generation: self.generation,
            total_ms: self.total_ms(),
            displays: self.expected.len(),
            placement_ok: self.placement_ok,
        })
    }

    /// Hotkey → the frame is on screen everywhere: the last paint report, or the last
    /// reveal where the window turns opaque only after the paint (Windows).
    fn total_ms(&self) -> f64 {
        self.t4_ms
            .values()
            .chain(self.t_reveal_ms.values())
            .copied()
            .fold(0.0_f64, f64::max)
    }

    fn t_reveal_max(&self) -> Option<f64> {
        self.t_reveal_ms.values().copied().reduce(f64::max)
    }

    /// The JSONL row for the completed generation, or `None` before completion.
    /// `run` is the per-process counter; `hw`/`os`/`power` come from [`LatencyLog`].
    pub fn row(&self, run: u64, hw: &str, os: &str, power: &str) -> Option<LatencyRow> {
        if !self.completed {
            return None;
        }
        Some(LatencyRow {
            schema: SCHEMA,
            run,
            scenario: self.context.scenario.clone(),
            ts: rfc3339_utc(epoch_secs_now()),
            hw: hw.to_string(),
            os: os.to_string(),
            power: power.to_string(),
            displays: self.context.displays.clone(),
            t0_epoch_us: u64::try_from(self.t0_epoch_us).unwrap_or(u64::MAX),
            t1_ms: self.t1_ms,
            t2_ms: string_keys(&self.t2_ms),
            t_show_ms: self.t_show_ms,
            t4_ms: string_keys(&self.t4_ms),
            t4_js_wall_ms: string_keys(&self.t4_js_wall_ms),
            t_reveal_ms: self.t_reveal_max(),
            total_ms: self.total_ms(),
            placement_ok: self.placement_ok.unwrap_or(false),
            selftest: self.context.selftest,
            fmt: self.context.fmt.clone(),
            notes: if self.t_show_ms.is_none() {
                "t_show missing".to_string()
            } else {
                String::new()
            },
        })
    }
}

fn string_keys(map: &BTreeMap<DisplayId, f64>) -> BTreeMap<String, f64> {
    map.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

/// Milliseconds elapsed since `t0`.
pub fn ms_since(t0: Instant) -> f64 {
    t0.elapsed().as_secs_f64() * 1000.0
}

fn epoch_secs_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Deliver a run outcome to the installed reporter, if any.
pub fn report(reporter: &Reporter, outcome: Result<RunReport, String>) {
    if let Some(tx) = reporter.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
        let _ = tx.send(outcome);
    }
}

// ---------------------------------------------------------------------------------------
// Log file, scenario tag, and machine probes
// ---------------------------------------------------------------------------------------

/// `~/Library/Logs/Snapshade` on macOS; `%LOCALAPPDATA%/Snapshade/Logs` on Windows (a plain
/// PowerShell launch has no `HOME`, which used to put the file in the working directory).
/// Falls back to the current directory when the variable is unset.
pub fn log_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    let (var, tail): (&str, &[&str]) = ("LOCALAPPDATA", &["Snapshade", "Logs"]);
    #[cfg(not(target_os = "windows"))]
    let (var, tail): (&str, &[&str]) = ("HOME", &["Library", "Logs", "Snapshade"]);
    match std::env::var_os(var) {
        Some(base) if !base.is_empty() => {
            tail.iter().fold(PathBuf::from(base), |p, seg| p.join(seg))
        }
        _ => PathBuf::from("."),
    }
}

/// `~/Library/Logs/Snapshade/spike-latency.jsonl`.
pub fn log_path() -> PathBuf {
    log_dir().join(LOG_FILE)
}

/// `~/Library/Logs/Snapshade/scenario.txt`.
pub fn scenario_path() -> PathBuf {
    log_dir().join(SCENARIO_FILE)
}

/// Resolve the scenario tag from the env var, else the tag file, else `untagged`.
/// Re-read on every call so the human can switch scenarios by editing the file.
pub fn scenario_tag() -> String {
    let from_env = std::env::var(SCENARIO_ENV).ok();
    let from_file = fs::read_to_string(scenario_path()).ok();
    scenario_tag_from(from_env.as_deref(), from_file.as_deref())
}

/// Pure precedence rule behind [`scenario_tag`]: env > file > `untagged`; both
/// sources are trimmed and an empty value falls through.
pub fn scenario_tag_from(env: Option<&str>, file: Option<&str>) -> String {
    for candidate in [env, file].into_iter().flatten() {
        let trimmed = candidate.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    UNTAGGED.to_string()
}

/// Append one row to the log (creating the directory) and mirror it to stderr as
/// `latency {json}`. The row has no pixel data by construction.
pub fn append_row(row: &LatencyRow) -> io::Result<()> {
    let json = serde_json::to_string(row).map_err(io::Error::other)?;
    eprintln!("latency {json}");
    let path = log_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    file.write_all(json.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}

/// Number of non-blank lines in the log (0 if it does not exist) — seeds the run counter.
pub fn existing_rows() -> u64 {
    match fs::File::open(log_path()) {
        Ok(file) => BufReader::new(file)
            .lines()
            .map_while(Result::ok)
            .filter(|l| !l.trim().is_empty())
            .count() as u64,
        Err(_) => 0,
    }
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// `sysctl -n hw.model` (e.g. `MacBookPro18,3`), or `unknown`.
pub fn hw_model() -> String {
    command_output("sysctl", &["-n", "hw.model"]).unwrap_or_else(|| "unknown".to_string())
}

/// `sw_vers -productVersion` + ` (` + `sw_vers -buildVersion` + `)`, or `unknown`.
pub fn os_version() -> String {
    match (
        command_output("sw_vers", &["-productVersion"]),
        command_output("sw_vers", &["-buildVersion"]),
    ) {
        (Some(product), Some(build)) => format!("{product} ({build})"),
        (Some(product), None) => product,
        _ => "unknown".to_string(),
    }
}

/// `pmset -g batt` → `battery` | `ac` | `unknown`.
pub fn power_source() -> String {
    command_output("pmset", &["-g", "batt"])
        .map(|text| power_from_pmset(&text))
        .unwrap_or_else(|| "unknown".to_string())
}

/// Pure classifier behind [`power_source`].
pub fn power_from_pmset(text: &str) -> String {
    if text.contains("Battery Power") {
        "battery".to_string()
    } else if text.contains("AC Power") {
        "ac".to_string()
    } else {
        "unknown".to_string()
    }
}

/// RFC 3339 UTC at second resolution from Unix seconds (no chrono: civil-from-days
/// per Howard Hinnant's algorithm).
pub fn rfc3339_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // days since 1970-01-01 → civil date
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if month <= 2 { y + 1 } else { y };
    format!("{year:04}-{month:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

/// Per-process log context (managed state): the run counter seeded from the file at
/// startup, and the `hw`/`os` probes taken once. `power` is probed per row because
/// the human unplugs the charger between scenarios.
#[derive(Debug)]
pub struct LatencyLog {
    next_run: AtomicU64,
    hw: String,
    os: String,
}

impl LatencyLog {
    /// Count the existing rows and probe the machine. Never fails: a missing log
    /// starts at run 1, a missing probe reads `unknown`.
    pub fn open() -> Self {
        let log = Self {
            next_run: AtomicU64::new(existing_rows() + 1),
            hw: hw_model(),
            os: os_version(),
        };
        eprintln!(
            "latency log {} (next run {}), hw={}, os={}",
            log_path().display(),
            log.next_run.load(Ordering::Relaxed),
            log.hw,
            log.os
        );
        log
    }

    /// Allocate the next run number.
    pub fn next_run(&self) -> u64 {
        self.next_run.fetch_add(1, Ordering::Relaxed)
    }

    pub fn hw(&self) -> &str {
        &self.hw
    }

    pub fn os(&self) -> &str {
        &self.os
    }
}

#[cfg(test)]
mod tests {
    use super::{
        power_from_pmset, rfc3339_utc, scenario_tag_from, DisplayRow, LatencyRow, RunContext,
        Timings, SCHEMA, UNTAGGED,
    };
    use std::time::Instant;

    fn context() -> RunContext {
        RunContext {
            scenario: "a-desktop".to_string(),
            selftest: false,
            fmt: "native".to_string(),
            displays: vec![DisplayRow {
                id: 1,
                scale: 2.0,
                px: [4112, 2658],
                pt: [2056.0, 1329.0],
                origin_pt: [0.0, 0.0],
            }],
        }
    }

    #[test]
    fn reports_once_when_every_display_has_painted_and_builds_the_row() {
        let mut t = Timings::default();
        let t0 = Instant::now();
        t.begin(3, t0, 1_000_000, 0.0, [1, 2], context());
        assert_eq!(t.generation(), 3);
        assert!(
            t.row(1, "hw", "os", "ac").is_none(),
            "no row before completion"
        );
        t.record_served(3, 1, 1_020_000);
        t.record_served(2, 2, 1_030_000); // stale generation: ignored
        t.record_shown(3, true);
        assert!(t.record_painted(3, 1, 1_050.0).is_none());
        assert!(t.record_painted(3, 9, 1_050.0).is_none(), "unknown display");
        let report = t.record_painted(3, 2, 1_060.0).expect("complete");
        assert_eq!(report.generation, 3);
        assert_eq!(report.displays, 2);
        assert_eq!(report.placement_ok, Some(true));
        assert!(report.total_ms >= 0.0);
        assert!(t.record_painted(3, 2, 1_070.0).is_none(), "reported once");

        let row: LatencyRow = t
            .row(7, "MacBookPro18,3", "26.5.2 (25F84)", "battery")
            .unwrap();
        assert_eq!(row.schema, SCHEMA);
        assert_eq!(row.run, 7);
        assert_eq!(row.scenario, "a-desktop");
        assert_eq!(row.t0_epoch_us, 1_000_000);
        assert_eq!(row.t1_ms, 0.0);
        assert_eq!(row.t2_ms.get("1"), Some(&20.0));
        assert!(!row.t2_ms.contains_key("2"));
        assert_eq!(row.t4_ms.len(), 2);
        let t_show = row.t_show_ms.expect("record_shown ran");
        assert!(row.t1_ms <= t_show);
        assert!(t_show <= row.total_ms);
        assert!(row.placement_ok);
        assert!(!row.selftest);
        assert_eq!(row.fmt, "native");
        assert_eq!(row.displays.len(), 1);
        assert_eq!(row.power, "battery");
        assert!(row.ts.ends_with('Z') && row.ts.len() == 20);

        // Every schema field is on the wire, and nothing that could carry pixel data.
        let json = serde_json::to_string(&row).unwrap();
        for key in [
            "\"schema\":1",
            "\"run\":7",
            "\"scenario\":\"a-desktop\"",
            "\"ts\":",
            "\"hw\":",
            "\"os\":",
            "\"power\":\"battery\"",
            "\"displays\":[{\"id\":1,\"scale\":2.0,\"px\":[4112,2658],\"pt\":[2056.0,1329.0],\"origin_pt\":[0.0,0.0]}]",
            "\"t0_epoch_us\":1000000",
            "\"t1_ms\":0.0",
            "\"t2_ms\":{\"1\":20.0}",
            "\"t_show_ms\":",
            "\"t4_ms\":{",
            "\"t4_js_wall_ms\":{",
            "\"total_ms\":",
            "\"placement_ok\":true",
            "\"selftest\":false",
            "\"fmt\":\"native\"",
            "\"notes\":\"\"",
        ] {
            assert!(json.contains(key), "missing {key} in {json}");
        }
        assert!(!json.contains("rgba\":["), "no byte arrays in the row");
    }

    #[test]
    fn a_row_without_record_shown_serialises_t_show_ms_null_and_notes_it() {
        let mut t = Timings::default();
        t.begin(4, Instant::now(), 2_000_000, 1.5, [1], context());
        t.record_painted(4, 1, 2_050.0).expect("complete");
        let row = t.row(8, "hw", "os", "ac").unwrap();
        assert_eq!(row.t_show_ms, None);
        assert_eq!(row.notes, "t_show missing");
        assert!(!row.placement_ok, "placement unknown counts as not ok");
        let json = serde_json::to_string(&row).unwrap();
        assert!(json.contains("\"t_show_ms\":null"), "{json}");
        assert!(json.contains("\"notes\":\"t_show missing\""), "{json}");
    }

    #[test]
    fn scenario_precedence_is_env_then_file_then_untagged() {
        assert_eq!(scenario_tag_from(Some("a-desktop"), Some("b")), "a-desktop");
        assert_eq!(
            scenario_tag_from(Some("  "), Some(" b-fullscreen-safari\n")),
            "b-fullscreen-safari"
        );
        assert_eq!(scenario_tag_from(None, Some("\n")), UNTAGGED);
        assert_eq!(scenario_tag_from(None, None), UNTAGGED);
    }

    #[test]
    fn power_classifier() {
        assert_eq!(
            power_from_pmset("Now drawing from 'Battery Power'\n -InternalBattery-0"),
            "battery"
        );
        assert_eq!(power_from_pmset("Now drawing from 'AC Power'"), "ac");
        assert_eq!(power_from_pmset(""), "unknown");
    }

    #[test]
    fn rfc3339_formatting() {
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339_utc(1_761_214_323), "2025-10-23T10:12:03Z");
    }
}
