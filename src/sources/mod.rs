//! Data sources. Each one reads something already on disk and returns
//! events plus notes about what it couldn't read. A source never panics
//! on missing or unreadable files.

use std::io::ErrorKind;

use crate::timeline::SourceResult;

pub mod dpkg;

/// Runs every source and returns their results, labelled by name.
pub fn collect_all(since: i64, offset: i64) -> Vec<(&'static str, SourceResult)> {
    vec![("dpkg", dpkg::collect(since, offset))]
}

/// Reads the given files. Missing files are skipped silently (the package
/// manager may not be installed); other errors become notes.
/// Returns a note if none of the files exist at all.
pub fn read_logs(source: &str, paths: &[&str]) -> (Vec<String>, Vec<String>) {
    let mut texts = Vec::new();
    let mut notes = Vec::new();
    let mut missing = 0;
    for path in paths {
        match std::fs::read(path) {
            // Logs may contain invalid UTF-8 (odd package descriptions); be lenient.
            Ok(bytes) => texts.push(String::from_utf8_lossy(&bytes).into_owned()),
            Err(e) if e.kind() == ErrorKind::NotFound => missing += 1,
            Err(e) if e.kind() == ErrorKind::PermissionDenied => {
                notes.push(format!("{source}: no permission to read {path}"))
            }
            Err(e) => notes.push(format!("{source}: can't read {path}: {e}")),
        }
    }
    if missing == paths.len() {
        notes.push(format!("{source}: no log found ({})", paths.join(", ")));
    }
    (texts, notes)
}
