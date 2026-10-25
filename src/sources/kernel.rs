//! Installed kernels, found from `/lib/modules/<version>`, which exists on
//! every mainstream distro.
//!
//! The install time is the ctime (inode change time) of the kernel image,
//! `/boot/vmlinuz-<version>` or `/lib/modules/<version>/vmlinuz`. We use
//! ctime rather than mtime because package managers keep the mtime from the
//! build machine, while ctime is set when the file lands on this disk.
//! Removed kernels leave nothing behind; the package logs cover those.
//!
//! When run against the live system we also say which kernel is running,
//! and flag kernels installed since boot, which take effect on the next reboot.

use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::sources::under;
use crate::timeline::{Event, SourceResult};

const MODULE_DIRS: &[&str] = &["/lib/modules", "/usr/lib/modules"];

pub fn collect(root: &Path, since: i64) -> SourceResult {
    let mut result = SourceResult::default();
    let Some(versions) = list_versions(root, &mut result.notes) else {
        return result;
    };
    let running = read_trimmed(root, "/proc/sys/kernel/osrelease");
    let boot = read_trimmed(root, "/proc/stat").and_then(|s| boot_time(&s));

    for version in versions {
        let Some(time) = install_time(root, &version) else {
            continue;
        };
        if time < since {
            continue;
        }
        let mut text = format!("installed {version}");
        if running.as_deref() == Some(version.as_str()) {
            text.push_str(" (running)");
        } else if running.is_some() && boot.is_some_and(|b| time > b) {
            text.push_str(" (not booted yet)");
        }
        result.events.push(Event {
            time,
            kind: "kernel",
            text,
        });
    }
    result
}

/// Kernel versions with a modules directory, from the first directory that exists.
fn list_versions(root: &Path, notes: &mut Vec<String>) -> Option<Vec<String>> {
    for dir in MODULE_DIRS {
        let full = under(root, dir);
        match std::fs::read_dir(&full) {
            Ok(entries) => {
                let mut v: Vec<String> = entries
                    .flatten()
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect();
                v.sort();
                return Some(v);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                notes.push(format!("kernel: can't read {}: {e}", full.display()));
                return None;
            }
        }
    }
    notes.push("kernel: no /lib/modules directory found".into());
    None
}

/// When this kernel version landed on disk (see the module docs). `None` if
/// there is no kernel image: removing a kernel package often leaves its
/// modules directory behind, and that must not look like an install.
fn install_time(root: &Path, version: &str) -> Option<i64> {
    let candidates = [
        format!("/boot/vmlinuz-{version}"),
        format!("/lib/modules/{version}/vmlinuz"),
        format!("/usr/lib/modules/{version}/vmlinuz"),
    ];
    candidates
        .iter()
        .find_map(|p| std::fs::metadata(under(root, p)).ok())
        .map(|m| m.ctime())
}

fn read_trimmed(root: &Path, path: &str) -> Option<String> {
    let s = std::fs::read_to_string(under(root, path)).ok()?;
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// Boot time from the `btime` line of `/proc/stat`.
fn boot_time(stat: &str) -> Option<i64> {
    stat.lines()
        .find_map(|l| l.strip_prefix("btime "))
        .and_then(|v| v.trim().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::TestRoot;

    #[test]
    fn parses_boot_time() {
        assert_eq!(
            boot_time("cpu  1 2 3\nbtime 1714554000\nprocesses 9\n"),
            Some(1714554000)
        );
        assert_eq!(boot_time("cpu 1\n"), None);
    }

    #[test]
    fn finds_kernels_and_marks_running() {
        let root = TestRoot::new("kernel");
        root.file("/lib/modules/6.12.4-amd64/modules.dep", "");
        root.file("/boot/vmlinuz-6.12.4-amd64", "");
        root.file("/lib/modules/6.12.5-amd64/modules.dep", "");
        root.file("/boot/vmlinuz-6.12.5-amd64", "");
        // Leftover from a removed kernel: modules dir but no image.
        root.file("/lib/modules/6.12.3-amd64/modules.dep", "");
        root.file("/proc/sys/kernel/osrelease", "6.12.4-amd64\n");
        // Booted at time 1, so both freshly created kernels count as newer.
        root.file("/proc/stat", "btime 1\n");

        let r = collect(&root.0, 0);
        let texts: Vec<&str> = r.events.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "installed 6.12.4-amd64 (running)",
                "installed 6.12.5-amd64 (not booted yet)",
            ]
        );
        assert!(r.notes.is_empty());

        // Everything was created just now, so a future `since` hides it all.
        assert!(collect(&root.0, i64::MAX).events.is_empty());
    }

    #[test]
    fn no_running_marker_without_proc() {
        // E.g. a disk mounted from a rescue system.
        let root = TestRoot::new("kernel-noproc");
        root.file("/usr/lib/modules/6.11.0/vmlinuz", "");
        let r = collect(&root.0, 0);
        assert_eq!(r.events[0].text, "installed 6.11.0");
    }

    #[test]
    fn notes_missing_modules_dir() {
        let root = TestRoot::new("kernel-empty");
        let r = collect(&root.0, 0);
        assert!(r.events.is_empty());
        assert_eq!(r.notes.len(), 1);
    }
}
