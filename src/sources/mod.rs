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

pub mod dpkg;
pub mod kernel;
pub mod shell;

/// Runs every source and returns their results, labelled by name.
pub fn collect_all(root: &Path, since: i64, tz: &Tz) -> Vec<(&'static str, SourceResult)> {
    vec![
        ("dpkg", dpkg::collect(root, since, tz)),
        ("kernel", kernel::collect(root, since)),
        ("shell", shell::collect(root, since)),
    ]
}

/// Joins an absolute system path like `/var/log/dpkg.log` onto `root`.
pub fn under(root: &Path, path: &str) -> PathBuf {
    root.join(path.trim_start_matches('/'))
}

/// Reads the given files below `root`. Missing files are skipped silently
/// (the package manager may not be installed); other errors become notes.
/// Returns a note if none of the files exist at all.
pub fn read_logs(source: &str, root: &Path, paths: &[&str]) -> (Vec<String>, Vec<String>) {
    let mut texts = Vec::new();
    let mut notes = Vec::new();
    let mut missing = 0;
    for path in paths {
        let full = under(root, path);
        match std::fs::read(&full) {
            // Logs may contain invalid UTF-8 (odd package descriptions); be lenient.
            Ok(bytes) => texts.push(String::from_utf8_lossy(&bytes).into_owned()),
            Err(e) if e.kind() == ErrorKind::NotFound => missing += 1,
            Err(e) if e.kind() == ErrorKind::PermissionDenied => notes.push(format!(
                "{source}: no permission to read {}",
                full.display()
            )),
            Err(e) => notes.push(format!("{source}: can't read {}: {e}", full.display())),
        }
    }
    if missing == paths.len() {
        let all: Vec<String> = paths
            .iter()
            .map(|p| under(root, p).display().to_string())
            .collect();
        notes.push(format!("{source}: no log found ({})", all.join(", ")));
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
    fn read_logs_reports_missing_and_reads_present() {
        let root = TestRoot::new("readlogs");
        let (texts, notes) = read_logs("t", &root.0, &["/a.log"]);
        assert!(texts.is_empty());
        assert_eq!(notes.len(), 1);
        root.file("/a.log", "hello");
        let (texts, notes) = read_logs("t", &root.0, &["/a.log", "/b.log"]);
        assert_eq!(texts, ["hello"]);
        assert!(notes.is_empty());
    }
}
