//! systemd units: units enabled or masked, and unit files or drop-ins edited.
//!
//! `systemctl enable foo.service` creates a symlink like
//! `/etc/systemd/system/multi-user.target.wants/foo.service`; the symlink's
//! own mtime (`symlink_metadata`) is when that happened. `systemctl mask`
//! links the unit to `/dev/null`. Regular files are local unit files and
//! drop-ins (`foo.service.d/override.conf`, what `systemctl edit` writes).
//!
//! We look in `/etc/systemd/system`, `/etc/systemd/user` and each user's
//! `~/.config/systemd/user`. The `config` source skips those trees so
//! nothing shows twice.
//!
//! Disabling a unit removes its symlink, which leaves nothing to date, so
//! that isn't shown.

use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::sources::{shell, under};
use crate::timeline::{Event, SourceResult};

/// Unit directories below /etc. Public so the `config` source can skip them.
pub const SYSTEM_DIRS: &[&str] = &["/etc/systemd/system", "/etc/systemd/user"];

/// The unit directory below each home.
pub const HOME_DIR: &str = ".config/systemd/user";

/// What happened to one unit, before grouping.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Change {
    Enabled(String),
    Masked(String),
    Edited(String),
}

pub fn collect(root: &Path, since: i64) -> SourceResult {
    let mut result = SourceResult::default();
    let mut changes = Vec::new();
    let mut dirs: Vec<(String, String)> = vec![
        (SYSTEM_DIRS[0].to_string(), String::new()),
        (SYSTEM_DIRS[1].to_string(), " (user)".to_string()),
    ];
    for home in shell::homes(root) {
        let user = home.rsplit('/').next().unwrap_or_default().to_string();
        dirs.push((format!("{home}/{HOME_DIR}"), format!(" (user {user})")));
    }
    for (dir, label) in dirs {
        scan(root, &dir, &label, since, &mut changes, &mut result.notes);
    }
    result.events = group(changes);
    result
}

/// Looks at one unit directory and its subdirectories (`*.wants/`,
/// `*.requires/`, `*.d/`), one level deep, which is all systemd uses.
fn scan(
    root: &Path,
    dir: &str,
    label: &str,
    since: i64,
    changes: &mut Vec<(i64, Change)>,
    notes: &mut Vec<String>,
) {
    for (path, name) in list(root, dir, notes) {
        let Ok(meta) = std::fs::symlink_metadata(under(root, &path)) else {
            continue;
        };
        if meta.is_dir() {
            let is_links = name.ends_with(".wants") || name.ends_with(".requires");
            for (sub, unit) in list(root, &path, notes) {
                let Ok(m) = std::fs::symlink_metadata(under(root, &sub)) else {
                    continue;
                };
                if m.mtime() < since {
                    continue;
                }
                if is_links && m.file_type().is_symlink() {
                    changes.push((m.mtime(), Change::Enabled(format!("{unit}{label}"))));
                } else if m.is_file() {
                    changes.push((m.mtime(), Change::Edited(sub)));
                }
            }
        } else if meta.mtime() >= since {
            if meta.file_type().is_symlink() {
                let target = std::fs::read_link(under(root, &path)).unwrap_or_default();
                if target == Path::new("/dev/null") {
                    changes.push((meta.mtime(), Change::Masked(format!("{name}{label}"))));
                }
                // Other top-level links are aliases (display-manager.service,
                // dbus-org.*); they come with an enable we already show.
            } else if meta.is_file() {
                changes.push((meta.mtime(), Change::Edited(path)));
            }
        }
    }
}

/// Sorted `(path, name)` entries of a directory. Missing is fine.
fn list(root: &Path, dir: &str, notes: &mut Vec<String>) -> Vec<(String, String)> {
    let full = under(root, dir);
    match std::fs::read_dir(&full) {
        Ok(entries) => {
            let mut v: Vec<(String, String)> = entries
                .flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .map(|name| (format!("{dir}/{name}"), name))
                .collect();
            v.sort();
            v
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Vec::new(),
        Err(e) if e.kind() == ErrorKind::PermissionDenied => {
            notes.push(format!(
                "service: no permission to look in {}",
                full.display()
            ));
            Vec::new()
        }
        Err(e) => {
            notes.push(format!("service: can't read {}: {e}", full.display()));
            Vec::new()
        }
    }
}

/// One line per minute and kind of change: "a.service, b.timer enabled".
/// A unit wanted by two targets is listed once.
fn group(mut changes: Vec<(i64, Change)>) -> Vec<Event> {
    changes.sort_by(|a, b| (a.0 / 60, &a.1).cmp(&(b.0 / 60, &b.1)));
    changes.dedup_by(|b, a| a.0 / 60 == b.0 / 60 && a.1 == b.1);
    let mut events: Vec<(i64, &'static str, Vec<String>)> = Vec::new();
    for (time, change) in changes {
        let (verb, name) = match change {
            Change::Enabled(n) => ("enabled", n),
            Change::Masked(n) => ("masked", n),
            Change::Edited(n) => ("edited", n),
        };
        match events.last_mut() {
            Some((t, v, names)) if *t / 60 == time / 60 && *v == verb => names.push(name),
            _ => events.push((time, verb, vec![name])),
        }
    }
    events
        .into_iter()
        .map(|(time, verb, names)| Event {
            time,
            kind: "service",
            text: format!("{} {verb}", names.join(", ")),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::TestRoot;
    use std::os::unix::fs::symlink;

    fn link(root: &TestRoot, target: &str, path: &str) {
        let full = under(&root.0, path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        symlink(target, full).unwrap();
    }

    #[test]
    fn finds_enabled_masked_and_edited_units() {
        let root = TestRoot::new("systemd");
        let sys = "/etc/systemd/system";
        root.file(&format!("{sys}/myapp.service"), "");
        root.file(&format!("{sys}/nginx.service.d/override.conf"), "");
        link(
            &root,
            "/lib/systemd/system/docker.service",
            &format!("{sys}/multi-user.target.wants/docker.service"),
        );
        link(&root, "/dev/null", &format!("{sys}/cups.service"));
        link(
            &root,
            "/lib/systemd/system/gdm.service",
            &format!("{sys}/display-manager.service"),
        );
        link(
            &root,
            "/usr/lib/systemd/user/backup.timer",
            "/home/alice/.config/systemd/user/timers.target.wants/backup.timer",
        );

        let r = collect(&root.0, 0);
        assert!(r.notes.is_empty());
        assert!(r.events.iter().all(|e| e.kind == "service"));
        let mut texts: Vec<String> = r.events.iter().map(|e| e.text.clone()).collect();
        texts.sort();
        // Everything was written within a second or two; it may straddle a
        // minute boundary, so check the parts rather than exact lines.
        let all = texts.join("\n");
        assert!(all.contains("docker.service"));
        assert!(all.contains("backup.timer (user alice)"));
        assert!(all.contains("cups.service masked"));
        assert!(all.contains("/etc/systemd/system/myapp.service"));
        assert!(all.contains("/etc/systemd/system/nginx.service.d/override.conf"));
        assert!(!all.contains("display-manager"));

        assert!(collect(&root.0, i64::MAX).events.is_empty());
    }

    #[test]
    fn groups_and_dedups_per_minute() {
        let changes = vec![
            (130, Change::Enabled("b.service".into())),
            (125, Change::Enabled("a.service".into())),
            (126, Change::Enabled("a.service".into())), // wanted by two targets
            (127, Change::Edited("/etc/systemd/system/a.service".into())),
            (200, Change::Masked("c.service".into())),
        ];
        let texts: Vec<(i64, String)> = group(changes)
            .into_iter()
            .map(|e| (e.time, e.text))
            .collect();
        assert_eq!(
            texts,
            [
                (125, "a.service, b.service enabled".to_string()),
                (127, "/etc/systemd/system/a.service edited".to_string()),
                (200, "c.service masked".to_string()),
            ]
        );
    }

    #[test]
    fn empty_root_is_quiet() {
        let root = TestRoot::new("systemd-empty");
        let r = collect(&root.0, 0);
        assert!(r.events.is_empty());
        assert!(r.notes.is_empty());
    }
}
