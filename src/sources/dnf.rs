//! Package changes from the dnf rpm log (Fedora, RHEL, CentOS Stream, ...).
//!
//! Lines look like:
//! `2024-05-01T09:12:00+0200 SUBDEBUG Upgrade: mesa-24.2-1.fc40.x86_64`
//! An upgrade writes two lines: `Upgrade:` with the new package, then
//! `Upgraded:` with the old one being cleaned up. We pair them so the output
//! reads "mesa 24.1-1.fc40 -> 24.2-1.fc40", like the other package logs.
//! dnf keeps the full history in a sqlite database, but reading that would
//! need a crate; this plain log is enough for recent changes.

use std::path::Path;

use crate::sources::pkg::{Action, group};
use crate::sources::read_logs;
use crate::timeline::{Event, SourceResult};
use crate::tz::Tz;

/// dnf rotates its log itself, to `dnf.rpm.log.1` and up.
pub const LOGS: &[&str] = &["/var/log/dnf.rpm.log.1", "/var/log/dnf.rpm.log"];

pub fn collect(root: &Path, since: i64, tz: &Tz) -> SourceResult {
    let (texts, notes) = read_logs("dnf", root, LOGS);
    let events = texts.iter().flat_map(|t| parse(t, since, tz)).collect();
    SourceResult { events, notes }
}

/// Splits `name-version-release.arch` into name and `version-release`.
/// Package names may contain dashes themselves (`xorg-x11-server-Xorg`), so we
/// split from the right.
fn split_nevra(s: &str) -> Option<(&str, &str)> {
    let nvr = s.rsplit_once('.').map_or(s, |(nvr, _arch)| nvr);
    let (nv, _release) = nvr.rsplit_once('-')?;
    let (name, _version) = nv.rsplit_once('-')?;
    Some((name, &nvr[name.len() + 1..]))
}

/// One line of the log: (time, what happened, name, version).
fn parse_line<'a>(line: &'a str, tz: &Tz) -> Option<(i64, &'a str, &'a str, &'a str)> {
    let (stamp, rest) = line.split_once(' ')?;
    let rest = rest.strip_prefix("SUBDEBUG ")?;
    let (what, pkg) = rest.split_once(": ")?;
    let (name, version) = split_nevra(pkg.trim())?;
    Some((tz.parse_stamp(stamp)?, what, name, version))
}

/// Parses log text, keeping actions at or after `since`, grouped per minute
/// like the other package sources.
pub fn parse(text: &str, since: i64, tz: &Tz) -> Vec<Event> {
    // (action, package name, old version still missing) in log order.
    let mut actions: Vec<(Action, &str, bool)> = Vec::new();
    for (time, what, name, version) in text.lines().filter_map(|l| parse_line(l, tz)) {
        let verb = match what {
            "Installed" | "Install" => "installed",
            "Upgrade" => "upgraded",
            "Downgrade" => "downgraded",
            "Reinstalled" | "Reinstall" => "reinstalled",
            "Erase" | "Erased" | "Removed" | "Obsoleted" => "removed",
            "Upgraded" | "Downgraded" => {
                // The old version of a package just upgraded or downgraded.
                // Look back only a little: it follows within the same run.
                let pending = actions
                    .iter_mut()
                    .rev()
                    .take(500)
                    .find(|(a, n, missing)| *missing && *n == name && time - a.time < 3600);
                if let Some((a, _, missing)) = pending {
                    a.detail = format!("{name} {version} -> {}", &a.detail[name.len() + 1..]);
                    *missing = false;
                }
                continue;
            }
            _ => continue,
        };
        // "Install" and "Installed" (or "Reinstall"/"Reinstalled") can both
        // appear for the same package, depending on the dnf version.
        let detail = format!("{name} {version}");
        if let Some((prev, _, _)) = actions.last()
            && prev.verb == verb
            && prev.detail == detail
            && time - prev.time < 3600
        {
            continue;
        }
        let waits_for_old = matches!(verb, "upgraded" | "downgraded");
        actions.push((Action { time, verb, detail }, name, waits_for_old));
    }
    group(actions.into_iter().map(|(a, _, _)| a), since)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
2024-05-01T09:12:00+0200 INFO --- logging initialized ---
2024-05-01T09:12:30+0200 SUBDEBUG Upgrade: mesa-dri-drivers-24.2-1.fc40.x86_64
2024-05-01T09:12:31+0200 SUBDEBUG Upgrade: kernel-core-6.12.5-200.fc40.x86_64
2024-05-01T09:12:40+0200 SUBDEBUG Upgraded: mesa-dri-drivers-24.1-3.fc40.x86_64
2024-05-01T09:12:41+0200 SUBDEBUG Upgraded: kernel-core-6.12.4-200.fc40.x86_64
2024-05-01T14:00:00+0200 SUBDEBUG Installed: htop-3.3.0-1.fc40.x86_64
2024-05-01T14:00:05+0200 SUBDEBUG Erase: nano-7.2-6.fc40.x86_64
2024-05-01T14:00:06+0200 SUBDEBUG Downgrade: vim-common-9.1-1.fc40.x86_64
2024-05-01T14:00:07+0200 SUBDEBUG Downgraded: vim-common-9.1-2.fc40.x86_64
2024-05-01T14:00:08+0200 SUBDEBUG some other line
";

    #[test]
    fn splits_name_version_release_arch() {
        assert_eq!(
            split_nevra("xorg-x11-server-Xorg-21.1.13-1.fc40.x86_64"),
            Some(("xorg-x11-server-Xorg", "21.1.13-1.fc40"))
        );
        assert_eq!(
            split_nevra("tzdata-2024a-5.fc40.noarch"),
            Some(("tzdata", "2024a-5.fc40"))
        );
        assert_eq!(split_nevra("nodashes.x86_64"), None);
    }

    #[test]
    fn parses_and_pairs_upgrades() {
        let events = parse(SAMPLE, 0, &Tz::utc());
        let texts: Vec<&str> = events.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "upgraded  mesa-dri-drivers 24.1-3.fc40 -> 24.2-1.fc40, \
                 kernel-core 6.12.4-200.fc40 -> 6.12.5-200.fc40",
                "installed htop 3.3.0-1.fc40",
                "removed   nano 7.2-6.fc40",
                "downgraded vim-common 9.1-2.fc40 -> 9.1-1.fc40",
            ]
        );
        // 09:12:30 at +0200 is 07:12:30 UTC.
        assert_eq!(events[0].time, 1714547550);
        // 2024-05-01 12:00 UTC: keeps only the afternoon actions.
        assert_eq!(parse(SAMPLE, 1714564800, &Tz::utc()).len(), 3);
    }

    #[test]
    fn upgrade_without_old_line_and_duplicate_install() {
        let log = "\
2024-05-01T09:00:00+0000 SUBDEBUG Upgrade: foo-2.0-1.fc40.x86_64
2024-05-01T10:00:00+0000 SUBDEBUG Install: bar-1.0-1.fc40.noarch
2024-05-01T10:00:01+0000 SUBDEBUG Installed: bar-1.0-1.fc40.noarch
";
        let events = parse(log, 0, &Tz::utc());
        let texts: Vec<&str> = events.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(
            texts,
            ["upgraded  foo 2.0-1.fc40", "installed bar 1.0-1.fc40"]
        );
    }

    #[test]
    fn collect_reads_log_from_root() {
        let root = crate::sources::TestRoot::new("dnf");
        let r = collect(&root.0, 0, &Tz::utc());
        assert!(r.events.is_empty() && r.notes.is_empty());
        root.file("/var/log/dnf.rpm.log", SAMPLE);
        assert_eq!(collect(&root.0, 0, &Tz::utc()).events.len(), 4);
    }
}
