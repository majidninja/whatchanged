//! System configuration: files under `/etc` whose contents changed.
//!
//! We walk `/etc` (not following symlinks, with a depth limit) and use each
//! regular file's mtime. Files the system rewrites by itself all the time
//! (`/etc/mtab`, `/etc/ld.so.cache`, ...) are skipped, and so are the shell
//! startup files, which the `shell` source already reports as `env`.
//!
//! Files edited in the same minute become one line, because one command
//! (an upgrade, `update-ca-certificates`, `useradd`) often rewrites several.

use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::sources::{shell, under};
use crate::timeline::{Event, SourceResult};

/// How many directory levels below /etc we look. Deeper trees are rare and
/// tend to be generated data, not hand-written config.
const MAX_DEPTH: usize = 6;

/// How many file names one line lists before "and N more".
const MAX_SHOWN: usize = 5;

/// Files that change on their own during normal operation.
const NOISY: &[&str] = &[
    "/etc/mtab",
    "/etc/ld.so.cache",
    "/etc/adjtime",
    "/etc/resolv.conf",
    "/etc/.pwd.lock",
    "/etc/passwd-",
    "/etc/group-",
    "/etc/shadow-",
    "/etc/gshadow-",
    "/etc/subuid-",
    "/etc/subgid-",
];

pub fn collect(root: &Path, since: i64) -> SourceResult {
    let mut result = SourceResult::default();
    let mut files = Vec::new();
    let mut denied = Vec::new();
    walk(root, "/etc", 0, since, &mut files, &mut denied, &mut result);
    if !denied.is_empty() {
        let more = match denied.len() {
            1 => String::new(),
            n => format!(" and {} more", n - 1),
        };
        result.notes.push(format!(
            "config: no permission to look in {}{more}",
            under(root, &denied[0]).display()
        ));
    }
    result.events = group(files);
    result
}

/// Collects `(mtime, path)` of regular files at or after `since` below `dir`.
fn walk(
    root: &Path,
    dir: &str,
    depth: usize,
    since: i64,
    files: &mut Vec<(i64, String)>,
    denied: &mut Vec<String>,
    result: &mut SourceResult,
) {
    let full = under(root, dir);
    let entries = match std::fs::read_dir(&full) {
        Ok(e) => e,
        Err(e) if e.kind() == ErrorKind::NotFound => return,
        Err(e) if e.kind() == ErrorKind::PermissionDenied => {
            denied.push(dir.to_string());
            return;
        }
        Err(e) => {
            result
                .notes
                .push(format!("config: can't read {}: {e}", full.display()));
            return;
        }
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    for name in names {
        let path = format!("{dir}/{name}");
        if is_skipped(&path) {
            continue;
        }
        // symlink_metadata: don't follow links (alternatives, certs, ...).
        let Ok(meta) = std::fs::symlink_metadata(under(root, &path)) else {
            continue;
        };
        if meta.is_dir() {
            if depth < MAX_DEPTH {
                walk(root, &path, depth + 1, since, files, denied, result);
            }
        } else if meta.is_file() && meta.mtime() >= since {
            files.push((meta.mtime(), path));
        }
    }
}

fn is_skipped(path: &str) -> bool {
    NOISY.contains(&path)
        || shell::SYSTEM_FILES.contains(&path)
        || shell::SYSTEM_DIRS.contains(&path)
}

/// Turns `(mtime, path)` pairs into events, one per minute.
fn group(mut files: Vec<(i64, String)>) -> Vec<Event> {
    files.sort();
    let mut groups: Vec<(i64, Vec<String>)> = Vec::new();
    for (time, path) in files {
        match groups.last_mut() {
            Some((t, paths)) if *t / 60 == time / 60 => paths.push(path),
            _ => groups.push((time, vec![path])),
        }
    }
    groups
        .into_iter()
        .map(|(time, mut paths)| {
            paths.sort();
            let mut text = paths[..paths.len().min(MAX_SHOWN)].join(", ");
            if paths.len() > MAX_SHOWN {
                text.push_str(&format!(" and {} more", paths.len() - MAX_SHOWN));
            }
            text.push_str(" edited");
            Event {
                time,
                kind: "config",
                text,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::TestRoot;

    #[test]
    fn finds_files_and_skips_noise() {
        let root = TestRoot::new("etc");
        root.file("/etc/fstab", "");
        root.file("/etc/ssh/sshd_config", "");
        root.file("/etc/mtab", ""); // noisy
        root.file("/etc/profile", ""); // belongs to the shell source
        root.file("/etc/profile.d/proxy.sh", ""); // same
        root.file("/home/alice/.bashrc", ""); // not under /etc

        // 2024-05-01 09:12:00 and :30 UTC: the same minute.
        set_mtime(&root, "/etc/fstab", 1714554720);
        set_mtime(&root, "/etc/ssh/sshd_config", 1714554750);

        let r = collect(&root.0, 0);
        assert!(r.notes.is_empty());
        let fstab_only = collect(&root.0, 1714554721);
        assert_eq!(fstab_only.events.len(), 1);
        assert_eq!(fstab_only.events[0].text, "/etc/ssh/sshd_config edited");

        let ours: Vec<&Event> = r.events.iter().filter(|e| e.time == 1714554720).collect();
        assert_eq!(ours.len(), 1);
        assert_eq!(ours[0].text, "/etc/fstab, /etc/ssh/sshd_config edited");
        assert_eq!(ours[0].kind, "config");
        // The noisy and shell files were written just now; they must not appear.
        assert!(
            r.events
                .iter()
                .all(|e| !e.text.contains("mtab") && !e.text.contains("profile"))
        );
    }

    fn set_mtime(root: &TestRoot, path: &str, secs: u64) {
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs);
        std::fs::File::options()
            .write(true)
            .open(under(&root.0, path))
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    #[test]
    fn does_not_follow_symlinks() {
        let root = TestRoot::new("etc-link");
        root.file("/usr/share/thing/a.conf", "");
        std::fs::create_dir_all(under(&root.0, "/etc")).unwrap();
        std::os::unix::fs::symlink(
            under(&root.0, "/usr/share/thing"),
            under(&root.0, "/etc/thing"),
        )
        .unwrap();
        assert!(collect(&root.0, 0).events.is_empty());
    }

    #[test]
    fn groups_by_minute() {
        let files = vec![
            (120, "/etc/b".to_string()),
            (61, "/etc/x".to_string()),
            (125, "/etc/a".to_string()),
            (130, "/etc/c".to_string()),
            (131, "/etc/d".to_string()),
            (132, "/etc/e".to_string()),
            (133, "/etc/f".to_string()),
            (134, "/etc/g".to_string()),
        ];
        let events = group(files);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].time, 61);
        assert_eq!(events[0].text, "/etc/x edited");
        assert_eq!(events[1].time, 120);
        assert_eq!(
            events[1].text,
            "/etc/a, /etc/b, /etc/c, /etc/d, /etc/e and 2 more edited"
        );
    }

    #[test]
    fn empty_root_is_quiet() {
        let root = TestRoot::new("etc-empty");
        let r = collect(&root.0, 0);
        assert!(r.events.is_empty());
        assert!(r.notes.is_empty());
    }
}
