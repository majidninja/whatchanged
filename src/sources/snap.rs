//! Snaps, from the squashfs files snapd keeps in `/var/lib/snapd/snaps`.
//!
//! Each installed revision is a file `<name>_<rev>.snap`. snapd writes a new
//! file on every install or refresh and keeps a couple of old revisions
//! around, so the newest revision's mtime is when the snap last changed, and
//! the next-newest revision (if still there) is what it was refreshed from.
//! Removals and reverts leave nothing we can see.
//!
//! A machine without snapd is normal, so a missing directory is silent.

use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::sources::pkg::{self, Action};
use crate::sources::under;
use crate::timeline::SourceResult;

const SNAPS: &str = "/var/lib/snapd/snaps";

pub fn collect(root: &Path, since: i64) -> SourceResult {
    let mut result = SourceResult::default();
    let dir = under(root, SNAPS);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return result,
        Err(e) => {
            result
                .notes
                .push(format!("snap: can't read {}: {e}", dir.display()));
            return result;
        }
    };
    // name -> [(mtime, revision)]
    let mut snaps: BTreeMap<String, Vec<(i64, String)>> = BTreeMap::new();
    for entry in entries.flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        let Some((name, rev)) = parse_name(&file) else {
            continue;
        };
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_file() {
            snaps
                .entry(name.to_string())
                .or_default()
                .push((meta.mtime(), rev.to_string()));
        }
    }
    let mut actions = Vec::new();
    for (name, mut revs) in snaps {
        revs.sort();
        let (time, new) = revs.pop().expect("at least one revision");
        if time < since {
            continue;
        }
        let detail = match revs.pop() {
            Some((_, old)) => format!("{name} {old} -> {new}"),
            None => format!("{name} rev {new}"),
        };
        actions.push(Action {
            time,
            verb: "snap",
            detail,
        });
    }
    actions.sort_by(|a, b| (a.time, &a.detail).cmp(&(b.time, &b.detail)));
    result.events = pkg::group(actions.into_iter(), since);
    result
}

/// Splits `core22_1380.snap` into ("core22", "1380"). Snap names can't
/// contain `_`, but revisions of locally installed snaps look like `x1`.
fn parse_name(file: &str) -> Option<(&str, &str)> {
    let (name, rev) = file.strip_suffix(".snap")?.rsplit_once('_')?;
    if name.is_empty() || rev.is_empty() || !rev.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some((name, rev))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::TestRoot;
    use std::time::{Duration, SystemTime};

    fn snap(root: &TestRoot, file: &str, mtime: u64) {
        let path = format!("{SNAPS}/{file}");
        root.file(&path, "");
        std::fs::File::options()
            .write(true)
            .open(under(&root.0, &path))
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(mtime))
            .unwrap();
    }

    #[test]
    fn parses_file_names() {
        assert_eq!(parse_name("core22_1380.snap"), Some(("core22", "1380")));
        assert_eq!(parse_name("hello_x1.snap"), Some(("hello", "x1")));
        assert_eq!(parse_name("firefox_4100.snap.partial"), None);
        assert_eq!(parse_name("_12.snap"), None);
        assert_eq!(parse_name("nounderscore.snap"), None);
    }

    #[test]
    fn newest_revision_with_previous() {
        let root = TestRoot::new("snap");
        snap(&root, "firefox_4100.snap", 1_000_000);
        snap(&root, "firefox_4200.snap", 2_000_000);
        snap(&root, "hello_42.snap", 2_000_010);
        snap(&root, "old_7.snap", 500);
        root.file(&format!("{SNAPS}/partial/x_1.snap.partial"), "");

        let r = collect(&root.0, 1_500_000);
        assert!(r.notes.is_empty());
        let texts: Vec<&str> = r.events.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["snap      firefox 4100 -> 4200, hello rev 42"]);
        assert_eq!(r.events[0].time, 2_000_000);
    }

    #[test]
    fn silent_without_snapd() {
        let root = TestRoot::new("snap-none");
        let r = collect(&root.0, 0);
        assert!(r.events.is_empty());
        assert!(r.notes.is_empty());
    }
}
