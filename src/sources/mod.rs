//! Data sources. Each one reads something already on disk and returns
//! events plus notes about what it couldn't read. A source never panics
//! on missing or unreadable files.
//!
//! Sources take a root directory (normally `/`) and look up their paths
//! below it, so they can be pointed at a mounted disk or a test directory.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::timeline::SourceResult;
use crate::tz::Tz;

pub mod cron;
pub mod dnf;
pub mod dpkg;
pub mod etc;
pub mod flatpak;
pub mod kernel;
pub mod pacman;
pub mod pkg;
pub mod shell;
pub mod snap;
pub mod systemd;

/// Every source as (name, kind of the events it produces). The order is
/// the order they run in.
pub const SOURCES: &[(&str, &str)] = &[
    ("pkg", "pkg"),
    ("flatpak", "pkg"),
    ("snap", "pkg"),
    ("kernel", "kernel"),
    ("config", "config"),
    ("shell", "env"),
    ("service", "service"),
    ("cron", "cron"),
];

/// Turns `--only pkg,config` into a list of source names. A word matches
/// a source by its name or by the kind shown in the timeline, so `pkg`
/// also picks flatpak and snap, and `env` picks the shell source.
pub fn select(only: &str) -> Result<Vec<&'static str>, String> {
    let mut chosen = Vec::new();
    for word in only.split(',').map(str::trim).filter(|w| !w.is_empty()) {
        let mut known = false;
        for &(name, kind) in SOURCES {
            if word == name || word == kind {
                known = true;
                if !chosen.contains(&name) {
                    chosen.push(name);
                }
            }
        }
        if !known {
            let names: Vec<&str> = SOURCES.iter().map(|s| s.0).collect();
            return Err(format!(
                "unknown source '{word}' (choose from: {}, env)",
                names.join(", ")
            ));
        }
    }
    if chosen.is_empty() {
        return Err("--only needs at least one source, e.g. --only pkg,config".into());
    }
    // Keep the usual run order, whatever order the user typed.
    chosen.sort_by_key(|n| SOURCES.iter().position(|s| s.0 == *n));
    Ok(chosen)
}

/// Runs the named sources and returns their results, labelled by name.
pub fn collect_all(
    root: &Path,
    since: i64,
    tz: &Tz,
    names: &[&'static str],
) -> Vec<(&'static str, SourceResult)> {
    names
        .iter()
        .map(|&name| {
            let result = match name {
                "pkg" => pkg::collect(root, since, tz),
                "flatpak" => flatpak::collect(root, since),
                "snap" => snap::collect(root, since),
                "kernel" => kernel::collect(root, since),
                "config" => etc::collect(root, since),
                "shell" => shell::collect(root, since),
                "service" => systemd::collect(root, since),
                "cron" => cron::collect(root, since),
                _ => unreachable!("unknown source {name}"),
            };
            (name, result)
        })
        .collect()
}

/// All source names, for when `--only` isn't given.
pub fn all_names() -> Vec<&'static str> {
    SOURCES.iter().map(|s| s.0).collect()
}

/// Joins an absolute system path like `/var/log/dpkg.log` onto `root`.
pub fn under(root: &Path, path: &str) -> PathBuf {
    root.join(path.trim_start_matches('/'))
}

/// Reads the given files below `root`. Missing files are skipped silently
/// (the package manager may not be installed); other errors become notes.
/// Whether *no* package log exists at all is reported by `pkg::collect`.
pub fn read_logs(source: &str, root: &Path, paths: &[&str]) -> (Vec<String>, Vec<String>) {
    let mut texts = Vec::new();
    let mut notes = Vec::new();
    for path in paths {
        let full = under(root, path);
        match std::fs::read(&full) {
            // Logs may contain invalid UTF-8 (odd package descriptions); be lenient.
            Ok(bytes) => texts.push(String::from_utf8_lossy(&bytes).into_owned()),
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) if e.kind() == ErrorKind::PermissionDenied => notes.push(format!(
                "{source}: no permission to read {}",
                full.display()
            )),
            Err(e) => notes.push(format!("{source}: can't read {}: {e}", full.display())),
        }
    }
    (texts, notes)
}

/// A scratch directory for tests that need files on disk. Removed on drop.
#[cfg(test)]
pub struct TestRoot(pub PathBuf);

#[cfg(test)]
impl TestRoot {
    pub fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("whatchanged-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TestRoot(dir)
    }

    /// Creates a file (and its parent directories) at `path` below the root.
    pub fn file(&self, path: &str, contents: &str) {
        let full = under(&self.0, path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, contents).unwrap();
    }
}

#[cfg(test)]
impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_joins_absolute_paths() {
        assert_eq!(
            under(Path::new("/mnt"), "/var/log/x"),
            PathBuf::from("/mnt/var/log/x")
        );
        assert_eq!(under(Path::new("/"), "/etc"), PathBuf::from("/etc"));
    }

    #[test]
    fn select_by_name_or_kind() {
        assert_eq!(
            select("config,pkg"),
            Ok(vec!["pkg", "flatpak", "snap", "config"])
        );
        assert_eq!(select("snap"), Ok(vec!["snap"]));
        assert_eq!(select("env, cron"), Ok(vec!["shell", "cron"]));
        assert_eq!(select("pkg,snap"), Ok(vec!["pkg", "flatpak", "snap"]));
        let err = select("pkg,docker").unwrap_err();
        assert!(err.contains("'docker'") && err.contains("flatpak"), "{err}");
        assert!(select(",").is_err());
    }

    #[test]
    fn select_names_all_have_a_collector() {
        // collect_all panics on a name it doesn't know; make sure every
        // listed source is wired up (on an empty root, so nothing is read).
        let root = TestRoot::new("selectall");
        let got = collect_all(&root.0, 0, &Tz::utc(), &all_names());
        assert_eq!(got.len(), SOURCES.len());
    }

    #[test]
    fn read_logs_skips_missing_and_reads_present() {
        let root = TestRoot::new("readlogs");
        let (texts, notes) = read_logs("t", &root.0, &["/a.log"]);
        assert!(texts.is_empty());
        assert!(notes.is_empty());
        root.file("/a.log", "hello");
        let (texts, notes) = read_logs("t", &root.0, &["/a.log", "/b.log"]);
        assert_eq!(texts, ["hello"]);
        assert!(notes.is_empty());
    }
}
