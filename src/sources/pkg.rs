//! Package changes from all package manager logs, plus the grouping they
//! share. Each package manager has its own module (dpkg, pacman, dnf); this one
//! runs them together so that a machine without, say, pacman doesn't get a
//! "no pacman log" note. Only if no package log exists at all do we say so.

use std::path::Path;

use crate::sources::{dnf, dpkg, pacman, under};
use crate::timeline::{Event, SourceResult};
use crate::tz::Tz;

/// Runs every package log source and merges their results.
pub fn collect(root: &Path, since: i64, tz: &Tz) -> SourceResult {
    let mut all = SourceResult::default();
    for r in [
        dpkg::collect(root, since, tz),
        pacman::collect(root, since, tz),
        dnf::collect(root, since, tz),
    ] {
        all.events.extend(r.events);
        all.notes.extend(r.notes);
    }
    all.notes.extend(unread_rotations(
        root,
        &[dpkg::LOGS, pacman::LOGS, dnf::LOGS].concat(),
        since,
    ));
    let logs: Vec<&str> = [dpkg::LOGS, pacman::LOGS, dnf::LOGS, &[dnf::DNF5_HISTORY]].concat();
    // Unreadable logs already produced a note of their own.
    if all.notes.is_empty() && !logs.iter().any(|p| under(root, p).exists()) {
        all.notes.push(format!(
            "pkg: no package manager log found (looked for {})",
            logs.iter()
                .filter(|p| !p.ends_with(".1"))
                .map(|p| under(root, p).display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    all
}

/// Rotated logs we don't read (`dpkg.log.2.gz`, `dnf.rpm.log.3`, ...)
/// that were last written at or after `since`, so they may hold changes in
/// the asked-for window. Only `.1` rotations are read: compressed ones would
/// need a gzip decoder, and older ones are rarely wanted. One note per log,
/// so nobody mistakes a gap for "nothing happened".
pub fn unread_rotations(root: &Path, logs: &[&str], since: i64) -> Vec<String> {
    use std::os::unix::fs::MetadataExt;
    let mut notes = Vec::new();
    for log in logs.iter().filter(|p| !p.ends_with(".1")) {
        let full = under(root, log);
        let (Some(dir), Some(base)) = (full.parent(), full.file_name()) else {
            continue;
        };
        let prefix = format!("{}.", base.to_string_lossy());
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut missed: Vec<String> = entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let read = logs.iter().any(|l| under(root, l) == e.path());
                let recent = e.metadata().is_ok_and(|m| m.mtime() >= since);
                (name.starts_with(&prefix) && !read && recent).then_some(name)
            })
            .collect();
        if missed.is_empty() {
            continue;
        }
        missed.sort();
        let shown = missed.len().min(3);
        let more = if missed.len() > shown {
            format!(" and {} more", missed.len() - shown)
        } else {
            String::new()
        };
        notes.push(format!(
            "pkg: older rotated logs in {} are not read ({}{more}); package changes in them are missing",
            dir.display(),
            missed[..shown].join(", ")
        ));
    }
    notes
}

/// One parsed package action, before grouping.
pub struct Action {
    pub time: i64,
    pub verb: &'static str,
    pub detail: String,
}

/// Packages listed per line before collapsing the rest into "and N more".
const MAX_SHOWN: usize = 5;

/// Turns package actions (in log order) into `pkg` events, merging actions
/// of the same kind within the same minute and dropping those before `since`.
/// A single `apt upgrade` or `pacman -Syu` usually touches many packages.
pub fn group(actions: impl Iterator<Item = Action>, since: i64) -> Vec<Event> {
    // (time of first action, verb, details) per group.
    let mut groups: Vec<(i64, &'static str, Vec<String>)> = Vec::new();
    for a in actions {
        if a.time < since {
            continue;
        }
        match groups.last_mut() {
            Some((t, verb, details)) if *t / 60 == a.time / 60 && *verb == a.verb => {
                details.push(a.detail)
            }
            _ => groups.push((a.time, a.verb, vec![a.detail])),
        }
    }
    groups
        .into_iter()
        .map(|(time, verb, details)| {
            let mut text = format!(
                "{verb:<9} {}",
                details[..details.len().min(MAX_SHOWN)].join(", ")
            );
            if details.len() > MAX_SHOWN {
                text.push_str(&format!(" and {} more", details.len() - MAX_SHOWN));
            }
            Event {
                time,
                kind: "pkg",
                text,
            }
        })
        .collect()
}

/// One run of a package manager command, from its own log
/// (apt's history.log, pacman's `[PACMAN] Running` lines).
#[derive(Debug, PartialEq)]
pub struct Run {
    pub start: i64,
    pub end: i64,
    pub command: String,
    pub user: Option<String>,
}

/// Longest command shown; long `apt-get -o ...` lines are cut.
pub const MAX_COMMAND: usize = 60;

pub fn shorten(command: &str) -> String {
    if command.chars().count() <= MAX_COMMAND {
        return command.to_string();
    }
    let cut: String = command.chars().take(MAX_COMMAND - 3).collect();
    format!("{cut}...")
}

/// Adds "(command, by user)" to events whose time falls inside a run.
pub fn annotate(events: &mut [Event], runs: &[Run]) {
    for e in events {
        if let Some(r) = runs.iter().find(|r| r.start <= e.time && e.time <= r.end) {
            match &r.user {
                Some(u) => e.text.push_str(&format!(" ({}, by {u})", r.command)),
                None => e.text.push_str(&format!(" ({})", r.command)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::TestRoot;

    #[test]
    fn one_note_when_no_package_log_exists() {
        let root = TestRoot::new("pkg-none");
        let r = collect(&root.0, 0, &Tz::utc());
        assert!(r.events.is_empty());
        assert_eq!(r.notes.len(), 1);
        assert!(r.notes[0].starts_with("pkg: no package manager log found"));
        assert!(r.notes[0].contains("pacman.log"));
        assert!(!r.notes[0].contains("dpkg.log.1"));
    }

    #[test]
    fn no_note_when_only_one_manager_is_used() {
        let root = TestRoot::new("pkg-pacman");
        root.file(
            "/var/log/pacman.log",
            "[2024-05-01T14:00:00+0200] [ALPM] installed htop (3.3.0-1)\n",
        );
        let r = collect(&root.0, 0, &Tz::utc());
        assert_eq!(r.events.len(), 1);
        assert!(r.notes.is_empty());

        let root = TestRoot::new("pkg-dpkg");
        root.file(
            "/var/log/dpkg.log",
            "2024-05-01 14:00:00 install htop:amd64 <none> 3.3.0-4\n",
        );
        let r = collect(&root.0, 0, &Tz::utc());
        assert_eq!(r.events.len(), 1);
        assert!(r.notes.is_empty());
    }

    #[test]
    fn no_note_when_only_dnf5_history_exists() {
        let root = TestRoot::new("pkg-dnf5");
        root.file(dnf::DNF5_HISTORY, "");
        let r = collect(&root.0, 0, &Tz::utc());
        assert_eq!(r.events.len(), 1);
        assert!(r.notes.is_empty());
    }

    #[test]
    fn notes_unread_rotations_written_since() {
        let root = TestRoot::new("pkg-rotations");
        root.file("/var/log/dpkg.log", "");
        root.file("/var/log/dpkg.log.1", "");
        root.file("/var/log/dpkg.log.2.gz", "");
        root.file("/var/log/dpkg.log.3.gz", "");
        root.file("/var/log/dpkg.logger", "");
        // All files are new, so with since = 0 both .gz are reported.
        let notes = unread_rotations(&root.0, dpkg::LOGS, 0);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(
            notes[0].contains("(dpkg.log.2.gz, dpkg.log.3.gz)"),
            "{}",
            notes[0]
        );
        // Written long before since: they can't hold anything we want.
        assert!(unread_rotations(&root.0, dpkg::LOGS, i64::MAX / 2).is_empty());
        // No note when nothing is rotated.
        let root = TestRoot::new("pkg-rotations-none");
        root.file("/var/log/dpkg.log", "");
        assert!(unread_rotations(&root.0, dpkg::LOGS, 0).is_empty());
    }

    #[test]
    fn groups_per_minute_and_kind() {
        let a = |time, verb, d: &str| Action {
            time,
            verb,
            detail: d.to_string(),
        };
        let events = group(
            [
                a(10, "installed", "old"),
                a(120, "installed", "a"),
                a(150, "installed", "b"),
                a(170, "removed", "c"),
                a(185, "removed", "d"),
            ]
            .into_iter(),
            100,
        );
        let texts: Vec<&str> = events.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["installed a, b", "removed   c", "removed   d"]);
    }
}
