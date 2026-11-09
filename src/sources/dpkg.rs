//! Package changes from the dpkg log (Debian, Ubuntu and friends).
//!
//! Lines look like:
//! `2024-05-01 09:12:33 upgrade mesa:amd64 24.1-1 24.2-1`
//! We only use install/upgrade/remove/purge; status and configure lines repeat
//! the same information.
//!
//! apt also keeps `/var/log/apt/history.log`, one block per run, saying which
//! command did it and who asked. When a dpkg change falls inside such a run,
//! the command is added: `upgraded  mesa 24.1 -> 24.2 (apt upgrade, by majid)`.

use std::path::Path;

use crate::sources::pkg::{Action, Run, annotate, group, shorten};
use crate::sources::read_logs;
use crate::time::DateTime;
use crate::timeline::{Event, SourceResult};
use crate::tz::Tz;

pub const LOGS: &[&str] = &["/var/log/dpkg.log.1", "/var/log/dpkg.log"];

/// apt's own log of runs. Optional: dpkg works fine without it.
pub const APT_HISTORY: &[&str] = &["/var/log/apt/history.log.1", "/var/log/apt/history.log"];

pub fn collect(root: &Path, since: i64, tz: &Tz) -> SourceResult {
    let (texts, mut notes) = read_logs("dpkg", root, LOGS);
    let mut events: Vec<Event> = texts.iter().flat_map(|t| parse(t, since, tz)).collect();
    let (history, apt_notes) = read_logs("apt", root, APT_HISTORY);
    notes.extend(apt_notes);
    let runs: Vec<Run> = history.iter().flat_map(|t| apt_runs(t, tz)).collect();
    annotate(&mut events, &runs);
    SourceResult { events, notes }
}

/// Parses history.log blocks:
/// ```text
/// Start-Date: 2024-05-01  09:12:20
/// Commandline: apt upgrade
/// Requested-By: majid (1000)
/// Upgrade: mesa:amd64 (24.1-1, 24.2-1)
/// End-Date: 2024-05-01  09:13:05
/// ```
/// Blocks without a command line are skipped. A block without an end
/// (apt was interrupted) is taken to last an hour.
fn apt_runs(text: &str, tz: &Tz) -> Vec<Run> {
    let time = |v: &str| {
        let v = v.split_whitespace().collect::<Vec<_>>().join(" ");
        DateTime::parse(&v).map(|dt| tz.to_epoch(dt))
    };
    let finish = |cur: &mut Option<Run>, runs: &mut Vec<Run>| {
        if let Some(r) = cur.take().filter(|r| !r.command.is_empty()) {
            runs.push(r);
        }
    };
    let mut runs = Vec::new();
    let mut cur: Option<Run> = None;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key {
            "Start-Date" => {
                finish(&mut cur, &mut runs);
                cur = time(value).map(|start| Run {
                    start,
                    end: start + 3600,
                    command: String::new(),
                    user: None,
                });
            }
            "Commandline" => {
                if let Some(r) = &mut cur {
                    r.command = shorten(value);
                }
            }
            "Requested-By" => {
                if let Some(r) = &mut cur {
                    // "majid (1000)" -> "majid"
                    let name = value.split(" (").next().unwrap_or(value).trim();
                    r.user = Some(name.to_string()).filter(|n| !n.is_empty());
                }
            }
            "End-Date" => {
                if let (Some(r), Some(end)) = (&mut cur, time(value)) {
                    r.end = end;
                }
                finish(&mut cur, &mut runs);
            }
            _ => {}
        }
    }
    finish(&mut cur, &mut runs);
    runs
}

fn parse_line(line: &str, tz: &Tz) -> Option<Action> {
    let mut f = line.split_whitespace();
    let date = f.next()?;
    let clock = f.next()?;
    let action = f.next()?;
    let pkg = f.next()?;
    let old = f.next()?;
    let new = f.next()?;
    let verb = match action {
        "install" => "installed",
        "upgrade" => "upgraded",
        "remove" => "removed",
        "purge" => "purged",
        _ => return None,
    };
    let time = tz.to_epoch(DateTime::parse(&format!("{date} {clock}"))?);
    // Drop the ":amd64" architecture suffix; it's noise for most people.
    let name = pkg.split(':').next().unwrap_or(pkg);
    let detail = match verb {
        "upgraded" => format!("{name} {old} -> {new}"),
        "installed" => format!("{name} {new}"),
        _ => format!("{name} {old}"),
    };
    Some(Action { time, verb, detail })
}

/// Parses log text, keeping actions at or after `since`. Actions of the same
/// kind within the same minute are merged into one event, because a single
/// `apt upgrade` usually touches many packages at once.
pub fn parse(text: &str, since: i64, tz: &Tz) -> Vec<Event> {
    group(text.lines().filter_map(|l| parse_line(l, tz)), since)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
2024-05-01 08:00:01 startup archives unpack
2024-05-01 09:12:30 upgrade mesa:amd64 24.1-1 24.2-1
2024-05-01 09:12:31 status half-configured mesa:amd64 24.2-1
2024-05-01 09:12:40 upgrade linux-image-amd64:amd64 6.12.4 6.12.5
2024-05-01 09:13:02 configure mesa:amd64 24.2-1 <none>
2024-05-01 14:00:00 install htop:amd64 <none> 3.3.0-4
2024-05-01 14:00:05 remove nano:amd64 7.2-1 <none>
garbage line
2024-05-01 99:00:00 install broken:amd64 <none> 1
";

    #[test]
    fn parses_and_groups() {
        let events = parse(SAMPLE, 0, &Tz::utc());
        let texts: Vec<&str> = events.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "upgraded  mesa 24.1-1 -> 24.2-1, linux-image-amd64 6.12.4 -> 6.12.5",
                "installed htop 3.3.0-4",
                "removed   nano 7.2-1",
            ]
        );
        assert_eq!(events[0].time, 1714554750);
    }

    #[test]
    fn collapses_long_groups() {
        let log: String = (0..8)
            .map(|i| format!("2024-05-01 10:00:0{i} install p{i}:amd64 <none> 1\n"))
            .collect();
        let events = parse(&log, 0, &Tz::utc());
        assert_eq!(
            events[0].text,
            "installed p0 1, p1 1, p2 1, p3 1, p4 1 and 3 more"
        );
    }

    #[test]
    fn respects_since() {
        // 2024-05-01 12:00 UTC
        let events = parse(SAMPLE, 1714564800, &Tz::utc());
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn collects_from_root() {
        let root = crate::sources::TestRoot::new("dpkg");
        root.file("/var/log/dpkg.log", SAMPLE);
        let r = collect(&root.0, 0, &Tz::utc());
        assert_eq!(r.events.len(), 3);
        assert!(r.notes.is_empty());
    }

    const HISTORY: &str = "
Start-Date: 2024-05-01  09:12:20
Commandline: apt upgrade
Requested-By: majid (1000)
Upgrade: mesa:amd64 (24.1-1, 24.2-1)
End-Date: 2024-05-01  09:13:05

Start-Date: 2024-05-01  13:59:58
Commandline: apt-get -y -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold install htop
Install: htop:amd64 (3.3.0-4)
End-Date: 2024-05-01  14:00:03

Start-Date: 2024-05-01  20:00:00
Upgrade: no-command:amd64 (1, 2)
End-Date: 2024-05-01  20:00:05
";

    #[test]
    fn parses_apt_history() {
        let runs = apt_runs(HISTORY, &Tz::utc());
        assert_eq!(runs.len(), 2);
        assert_eq!(
            runs[0],
            Run {
                start: 1714554740,
                end: 1714554785,
                command: "apt upgrade".into(),
                user: Some("majid".into()),
            }
        );
        assert_eq!(runs[1].user, None);
        assert!(runs[1].command.ends_with("..."));
        assert_eq!(
            runs[1].command.chars().count(),
            crate::sources::pkg::MAX_COMMAND
        );
    }

    #[test]
    fn interrupted_run_lasts_an_hour() {
        let text = "Start-Date: 2024-05-01  09:00:00\nCommandline: apt install x\n";
        let runs = apt_runs(text, &Tz::utc());
        assert_eq!(runs[0].end - runs[0].start, 3600);
    }

    #[test]
    fn adds_command_to_dpkg_events() {
        let root = crate::sources::TestRoot::new("dpkg-apt");
        root.file("/var/log/dpkg.log", SAMPLE);
        root.file("/var/log/apt/history.log", HISTORY);
        let r = collect(&root.0, 0, &Tz::utc());
        let texts: Vec<&str> = r.events.iter().map(|e| e.text.as_str()).collect();
        assert!(
            texts[0].ends_with(" (apt upgrade, by majid)"),
            "{}",
            texts[0]
        );
        assert!(texts[1].starts_with("installed htop 3.3.0-4 (apt-get -y"));
        // The removal at 14:00:05 is after that run ended.
        assert_eq!(texts[2], "removed   nano 7.2-1");
        assert!(r.notes.is_empty());
    }
}
