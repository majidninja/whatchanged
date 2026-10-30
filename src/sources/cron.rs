//! Crontabs: `/etc/crontab`, `/etc/cron.d/*`, the `/etc/cron.daily/*`
//! style script folders, and each user's own crontab (what `crontab -e`
//! writes).
//!
//! User crontabs live in `/var/spool/cron/crontabs/<user>` on Debian and
//! Ubuntu, and in `/var/spool/cron/<user>` on Fedora and Arch. Those
//! folders are usually only readable by root, so without root we say we
//! skipped them. The `config` source skips the `/etc` paths here so
//! nothing shows twice.
//!
//! A crontab removed with `crontab -r` leaves nothing to date, so removals
//! aren't shown.

use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::sources::under;
use crate::timeline::{Event, SourceResult};

/// Public so the `config` source can skip them.
pub const SYSTEM_FILES: &[&str] = &["/etc/crontab", "/etc/anacrontab"];

/// Public so the `config` source can skip them.
pub const SYSTEM_DIRS: &[&str] = &[
    "/etc/cron.d",
    "/etc/cron.hourly",
    "/etc/cron.daily",
    "/etc/cron.weekly",
    "/etc/cron.monthly",
];

/// User crontab folders; each file in them is named after its user.
const SPOOL_DIRS: &[&str] = &["/var/spool/cron/crontabs", "/var/spool/cron"];

pub fn collect(root: &Path, since: i64) -> SourceResult {
    let mut result = SourceResult::default();
    let mut changes: Vec<(i64, String)> = Vec::new();

    let mut files: Vec<String> = SYSTEM_FILES.iter().map(|s| s.to_string()).collect();
    for dir in SYSTEM_DIRS {
        files.extend(list(root, dir, &mut result.notes));
    }
    for path in files {
        if let Some(t) = file_mtime(root, &path)
            && t >= since
            && !is_placeholder(&path)
        {
            changes.push((t, path));
        }
    }

    for dir in SPOOL_DIRS {
        for path in list(root, dir, &mut result.notes) {
            if let Some(t) = file_mtime(root, &path)
                && t >= since
            {
                let user = path.rsplit('/').next().unwrap_or_default();
                changes.push((t, format!("crontab of {user}")));
            }
        }
    }

    result.events = group(changes);
    result
}

/// Debian ships a `.placeholder` file in each cron folder.
fn is_placeholder(path: &str) -> bool {
    path.ends_with("/.placeholder")
}

/// The mtime of a regular file (not following symlinks); `None` for
/// anything else, including folders like `/var/spool/cron/crontabs`.
fn file_mtime(root: &Path, path: &str) -> Option<i64> {
    let meta = std::fs::symlink_metadata(under(root, path)).ok()?;
    meta.is_file().then(|| meta.mtime())
}

/// Sorted paths in a directory. Missing is fine; no permission is a note.
fn list(root: &Path, dir: &str, notes: &mut Vec<String>) -> Vec<String> {
    let full = under(root, dir);
    match std::fs::read_dir(&full) {
        Ok(entries) => {
            let mut v: Vec<String> = entries
                .flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .map(|name| format!("{dir}/{name}"))
                .collect();
            v.sort();
            v
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Vec::new(),
        Err(e) if e.kind() == ErrorKind::PermissionDenied => {
            notes.push(format!(
                "cron: no permission to look in {} (user crontabs need root)",
                full.display()
            ));
            Vec::new()
        }
        Err(e) => {
            notes.push(format!("cron: can't read {}: {e}", full.display()));
            Vec::new()
        }
    }
}

/// One line per minute: "/etc/cron.d/certbot, crontab of alice edited".
fn group(mut changes: Vec<(i64, String)>) -> Vec<Event> {
    changes.sort_by(|a, b| (a.0 / 60, &a.1).cmp(&(b.0 / 60, &b.1)));
    let mut events: Vec<(i64, Vec<String>)> = Vec::new();
    for (time, name) in changes {
        match events.last_mut() {
            Some((t, names)) if *t / 60 == time / 60 => names.push(name),
            _ => events.push((time, vec![name])),
        }
    }
    events
        .into_iter()
        .map(|(time, names)| Event {
            time,
            kind: "cron",
            text: format!("{} edited", names.join(", ")),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::TestRoot;

    #[test]
    fn finds_system_and_user_crontabs() {
        let root = TestRoot::new("cron");
        root.file("/etc/crontab", "");
        root.file("/etc/cron.d/certbot", "");
        root.file("/etc/cron.daily/.placeholder", "");
        root.file("/etc/cron.daily/logrotate", "");
        root.file("/var/spool/cron/crontabs/alice", ""); // Debian
        root.file("/var/spool/cron/bob", ""); // Fedora, Arch

        let r = collect(&root.0, 0);
        assert!(r.notes.is_empty());
        assert!(r.events.iter().all(|e| e.kind == "cron"));
        let all: Vec<String> = r.events.iter().map(|e| e.text.clone()).collect();
        let all = all.join("\n");
        assert!(all.contains("/etc/crontab"));
        assert!(all.contains("/etc/cron.d/certbot"));
        assert!(all.contains("/etc/cron.daily/logrotate"));
        assert!(all.contains("crontab of alice"));
        assert!(all.contains("crontab of bob"));
        assert!(!all.contains("placeholder"));
        // The Debian spool folder itself is not a user called "crontabs".
        assert!(!all.contains("crontab of crontabs"));

        assert!(collect(&root.0, i64::MAX).events.is_empty());
    }

    #[test]
    fn groups_per_minute() {
        let changes = vec![
            (130, "crontab of alice".to_string()),
            (125, "/etc/crontab".to_string()),
            (200, "/etc/cron.d/certbot".to_string()),
        ];
        let texts: Vec<(i64, String)> = group(changes)
            .into_iter()
            .map(|e| (e.time, e.text))
            .collect();
        assert_eq!(
            texts,
            [
                (125, "/etc/crontab, crontab of alice edited".to_string()),
                (200, "/etc/cron.d/certbot edited".to_string()),
            ]
        );
    }

    #[test]
    fn empty_root_is_quiet() {
        let root = TestRoot::new("cron-empty");
        let r = collect(&root.0, 0);
        assert!(r.events.is_empty());
        assert!(r.notes.is_empty());
    }
}
