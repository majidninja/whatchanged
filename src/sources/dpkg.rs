//! Package changes from the dpkg log (Debian, Ubuntu and friends).
//!
//! Lines look like:
//! `2024-05-01 09:12:33 upgrade mesa:amd64 24.1-1 24.2-1`
//! We only use install/upgrade/remove/purge; status and configure lines repeat
//! the same information.

use crate::sources::read_logs;
use crate::time::DateTime;
use crate::timeline::{Event, SourceResult};
use crate::tz::Tz;

const LOGS: &[&str] = &["/var/log/dpkg.log.1", "/var/log/dpkg.log"];

pub fn collect(since: i64, tz: &Tz) -> SourceResult {
    let (texts, notes) = read_logs("dpkg", LOGS);
    let events = texts.iter().flat_map(|t| parse(t, since, tz)).collect();
    SourceResult { events, notes }
}

/// One parsed action, before grouping.
struct Action {
    time: i64,
    verb: &'static str,
    detail: String,
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
    // (time of first action, verb, details) per group.
    let mut groups: Vec<(i64, &'static str, Vec<String>)> = Vec::new();
    for a in text.lines().filter_map(|l| parse_line(l, tz)) {
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

/// Packages listed per line before collapsing the rest into "and N more".
const MAX_SHOWN: usize = 5;

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
}
