//! Save-file names: `Screenshot at <MMM dd HH-mm-ss>.<ext>` (docs/SPEC.md §5), made
//! collision-safe (EXPORT-03):
//! two saves within one second get ` (2)`, ` (3)`, … appended before the extension.
//! Pure: the caller supplies the local-time fields and the directory listing probe.

use std::path::{Path, PathBuf};

/// Three-letter English month abbreviations as `NSDateFormatter`'s `MMM` renders them.
pub const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Local-time fields of the save moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    /// 1..=12
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

/// `Screenshot at Oct 22 14-05-33` (no extension).
pub fn screenshot_stem(t: LocalTime) -> String {
    let month = MONTHS
        .get(t.month.saturating_sub(1) as usize)
        .copied()
        .unwrap_or("Jan");
    format!(
        "Screenshot at {month} {:02} {:02}-{:02}-{:02}",
        t.day, t.hour, t.minute, t.second
    )
}

/// `<dir>/<stem>.<ext>`, or `<dir>/<stem> (n).<ext>` for the smallest `n >= 2` whose path
/// `exists` reports free. `exists` is injected so the rule is testable without a disk.
pub fn unique_path(dir: &Path, stem: &str, ext: &str, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let first = dir.join(format!("{stem}.{ext}"));
    if !exists(&first) {
        return first;
    }
    for n in 2u32.. {
        let candidate = dir.join(format!("{stem} ({n}).{ext}"));
        if !exists(&candidate) {
            return candidate;
        }
    }
    unreachable!("u32 range exhausted")
}

/// `unique_path` against the real filesystem.
pub fn unique_path_on_disk(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    unique_path(dir, stem, ext, |p| p.exists())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{screenshot_stem, unique_path, LocalTime};

    const T: LocalTime = LocalTime {
        month: 10,
        day: 22,
        hour: 14,
        minute: 5,
        second: 33,
    };

    #[test]
    fn stem_follows_the_screenshot_at_pattern() {
        assert_eq!(screenshot_stem(T), "Screenshot at Oct 22 14-05-33");
        assert_eq!(
            screenshot_stem(LocalTime {
                month: 1,
                day: 3,
                hour: 0,
                minute: 0,
                second: 0
            }),
            "Screenshot at Jan 03 00-00-00"
        );
    }

    #[test]
    fn unique_path_appends_a_counter_only_on_collision() {
        let dir = Path::new("/tmp/shots");
        let taken: Vec<PathBuf> = vec![
            dir.join("Screenshot at Oct 22 14-05-33.png"),
            dir.join("Screenshot at Oct 22 14-05-33 (2).png"),
        ];
        let exists = |p: &Path| taken.iter().any(|t| t == p);
        assert_eq!(
            unique_path(dir, &screenshot_stem(T), "png", |_| false),
            dir.join("Screenshot at Oct 22 14-05-33.png")
        );
        assert_eq!(
            unique_path(dir, &screenshot_stem(T), "png", exists),
            dir.join("Screenshot at Oct 22 14-05-33 (3).png")
        );
        assert_eq!(
            unique_path(dir, &screenshot_stem(T), "jpg", exists),
            dir.join("Screenshot at Oct 22 14-05-33.jpg")
        );
    }
}
