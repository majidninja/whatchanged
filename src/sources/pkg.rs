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
    let logs: Vec<&str> = [dpkg::LOGS, pacman::LOGS, dnf::LOGS, &[dnf::DNF5_HISTORY]].concat();
    // Unreadable logs already produced a note of their own, and events
    // mean some log (perhaps only a rotated one) was read.
    if all.notes.is_empty()
        && all.events.is_empty()
        && !logs.iter().any(|p| under(root, p).exists())
    {
        all.notes.push(format!(
            "pkg: no package manager log found (looked for {})",
            logs.iter()
                .map(|p| under(root, p).display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    all
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
    fn reads_compressed_rotations_of_every_package_log() {
        // "2024-05-01 14:00:00 install htop:amd64 <none> 3.3.0-4\n" as gzip.
        let gz: &[u8] = &[
            0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x03, 0x33, 0x32, 0x30, 0x32,
            0xd1, 0x35, 0x30, 0xd5, 0x35, 0x30, 0x54, 0x30, 0x34, 0xb1, 0x32, 0x30, 0x00, 0x22,
            0x85, 0xcc, 0xbc, 0xe2, 0x92, 0xc4, 0x9c, 0x1c, 0x85, 0x8c, 0x92, 0xfc, 0x02, 0xab,
            0xc4, 0xdc, 0x14, 0x33, 0x13, 0x05, 0x9b, 0xbc, 0xfc, 0xbc, 0x54, 0x3b, 0x05, 0x63,
            0x3d, 0x63, 0x3d, 0x03, 0x5d, 0x13, 0x2e, 0x00, 0xe2, 0x0c, 0x08, 0x8d, 0x36, 0x00,
            0x00, 0x00,
        ];
        let root = TestRoot::new("pkg-gz");
        root.bytes("/var/log/dpkg.log.2.gz", gz);
        let r = collect(&root.0, 0, &Tz::utc());
        assert!(r.notes.is_empty(), "{:?}", r.notes);
        assert_eq!(r.events.len(), 1);
        assert_eq!(r.events[0].text, "installed htop 3.3.0-4");
        // Written before --since: not even opened.
        root.set_mtime("/var/log/dpkg.log.2.gz", 1000);
        let r = collect(&root.0, 2000, &Tz::utc());
        assert!(r.events.is_empty());
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
