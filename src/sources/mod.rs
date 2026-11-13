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

/// Largest size a compressed log may unpack to. Real package logs are a
/// few MB; this only guards against a broken or hostile file.
const MAX_UNPACKED: usize = 256 << 20;

/// Reads the given logs below `root`, with their rotated copies
/// (`dpkg.log.1`, `dpkg.log.2.gz`, ...) that were written at or after
/// `since`; older ones can't hold anything in the window. Missing files
/// are skipped silently (the package manager may not be installed); other
/// errors become notes. Whether *no* package log exists at all is reported
/// by `pkg::collect`.
pub fn read_logs(
    source: &str,
    root: &Path,
    paths: &[&str],
    since: i64,
) -> (Vec<String>, Vec<String>) {
    let mut texts = Vec::new();
    let mut notes = Vec::new();
    for path in paths {
        let full = under(root, path);
        let (mut files, unsupported) = rotations(&full, since);
        for name in unsupported {
            notes.push(format!(
                "{source}: {} is not read (only plain and .gz rotated logs are)",
                name.display()
            ));
        }
        files.push(full);
        for file in files {
            let bytes = match std::fs::read(&file) {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == ErrorKind::NotFound => continue,
                Err(e) if e.kind() == ErrorKind::PermissionDenied => {
                    notes.push(format!(
                        "{source}: no permission to read {}",
                        file.display()
                    ));
                    continue;
                }
                Err(e) => {
                    notes.push(format!("{source}: can't read {}: {e}", file.display()));
                    continue;
                }
            };
            let bytes = if file.extension().is_some_and(|x| x == "gz") {
                match crate::gzip::gunzip(&bytes, MAX_UNPACKED) {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        notes.push(format!("{source}: can't unpack {}: {e}", file.display()));
                        continue;
                    }
                }
            } else {
                bytes
            };
            // Logs may contain invalid UTF-8 (odd package descriptions); be lenient.
            texts.push(String::from_utf8_lossy(&bytes).into_owned());
        }
    }
    (texts, notes)
}

/// The rotated copies of `log` last written at or after `since`, oldest
/// first, as (readable ones, ones in a compression we can't read).
/// logrotate names them `x.log.1`, `x.log.2.gz`, or with a date:
/// `x.log-20240501.gz`.
fn rotations(log: &Path, since: i64) -> (Vec<PathBuf>, Vec<PathBuf>) {
    use std::os::unix::fs::MetadataExt;
    let (Some(dir), Some(base)) = (log.parent(), log.file_name()) else {
        return (Vec::new(), Vec::new());
    };
    let base = base.to_string_lossy();
    let mut readable = Vec::new();
    let mut unsupported = Vec::new();
    // An unreadable directory shows up when the log itself is read.
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(rest) = name
            .strip_prefix(&*base)
            .and_then(|r| r.strip_prefix(['.', '-']))
        else {
            continue;
        };
        let (number, packing) = match rest.rsplit_once('.') {
            Some((n, ext)) if !ext.bytes().all(|b| b.is_ascii_digit()) => (n, ext),
            _ => (rest, ""),
        };
        let numbered =
            !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit() || b == b'-');
        if !numbered {
            continue; // dpkg.log.bak, dpkg.logger: not ours
        }
        let Ok(meta) = e.metadata() else { continue };
        if !meta.is_file() || meta.mtime() < since {
            continue;
        }
        match packing {
            "" => readable.push((meta.mtime(), e.path())),
            // Both x.1 and x.1.gz (an interrupted rotation): read one.
            "gz" if dir.join(format!("{base}.{number}")).exists() => {}
            "gz" => readable.push((meta.mtime(), e.path())),
            "xz" | "bz2" | "zst" | "lz4" | "lzo" | "Z" => unsupported.push(e.path()),
            _ => {}
        }
    }
    readable.sort();
    unsupported.sort();
    (readable.into_iter().map(|(_, p)| p).collect(), unsupported)
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
        self.bytes(path, contents.as_bytes());
    }

    pub fn bytes(&self, path: &str, contents: &[u8]) {
        let full = under(&self.0, path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, contents).unwrap();
    }

    /// Sets a file's modification time (seconds since the epoch).
    pub fn set_mtime(&self, path: &str, time: i64) {
        let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(time as u64);
        std::fs::File::options()
            .write(true)
            .open(under(&self.0, path))
            .unwrap()
            .set_modified(when)
            .unwrap();
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
        let (texts, notes) = read_logs("t", &root.0, &["/a.log"], 0);
        assert!(texts.is_empty());
        assert!(notes.is_empty());
        root.file("/a.log", "hello");
        let (texts, notes) = read_logs("t", &root.0, &["/a.log", "/b.log"], 0);
        assert_eq!(texts, ["hello"]);
        assert!(notes.is_empty());
    }

    /// "old\n" as gzip.
    const OLD_GZ: &[u8] = &[
        0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x03, 0xcb, 0xcf, 0x49, 0xe1, 0x02,
        0x00, 0xb0, 0x4d, 0x88, 0xe2, 0x04, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn read_logs_reads_rotations_written_since() {
        let root = TestRoot::new("rotations");
        root.file("/a.log", "now\n");
        root.file("/a.log.1", "last\n");
        root.bytes("/a.log.2.gz", OLD_GZ);
        root.bytes("/a.log.3.gz", OLD_GZ);
        root.file("/a.log.bak", "no\n");
        root.file("/a.logger", "no\n");
        root.file("/a.log.4.xz", "?");
        root.set_mtime("/a.log.1", 3000);
        root.set_mtime("/a.log.2.gz", 2000);
        root.set_mtime("/a.log.3.gz", 1000);
        root.set_mtime("/a.log.4.xz", 500);
        // Oldest first, the current log last.
        let (texts, notes) = read_logs("t", &root.0, &["/a.log"], 0);
        assert_eq!(texts, ["old\n", "old\n", "last\n", "now\n"]);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("a.log.4.xz is not read"), "{}", notes[0]);
        // Rotations last written before `since` are left alone, and so is
        // the note about the one we couldn't have read.
        let (texts, notes) = read_logs("t", &root.0, &["/a.log"], 2500);
        assert_eq!(texts, ["last\n", "now\n"]);
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn read_logs_handles_dated_broken_and_doubled_rotations() {
        let root = TestRoot::new("rotations-odd");
        root.bytes("/a.log-20240501.gz", OLD_GZ);
        root.bytes("/a.log.2.gz", &OLD_GZ[..OLD_GZ.len() - 3]);
        root.file("/a.log.1", "last\n");
        root.bytes("/a.log.1.gz", OLD_GZ);
        root.set_mtime("/a.log-20240501.gz", 1000);
        root.set_mtime("/a.log.1", 2000);
        // No current log: the rotations are still read.
        let (texts, notes) = read_logs("t", &root.0, &["/a.log"], 0);
        assert_eq!(texts, ["old\n", "last\n"]);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].starts_with("t: can't unpack"), "{}", notes[0]);
        assert!(notes[0].contains("a.log.2.gz"), "{}", notes[0]);
    }
}
