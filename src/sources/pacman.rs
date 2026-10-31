//! Package changes from the pacman log (Arch, Manjaro, EndeavourOS, ...).
//!
//! Lines look like:
//! `[2024-05-01T09:12:00+0200] [ALPM] upgraded mesa (1:24.1-1 -> 1:24.2-1)`
//! Older logs use local time without an offset, `[2019-01-01 12:00]`, and
//! very old ones have no `[ALPM]` tag. We keep only package actions; the
//! `[PACMAN]` command lines and `[ALPM-SCRIPTLET]` output are skipped.

use std::path::Path;

use crate::sources::dpkg::{Action, group};
use crate::sources::read_logs;
use crate::time::{DateTime, HOUR, MINUTE};
use crate::timeline::{Event, SourceResult};
use crate::tz::Tz;

const LOGS: &[&str] = &["/var/log/pacman.log"];

pub fn collect(root: &Path, since: i64, tz: &Tz) -> SourceResult {
    let (texts, mut notes) = read_logs("pacman", root, LOGS);
    // Most machines don't use pacman; a missing log isn't worth a note.
    if texts.is_empty() {
        notes.retain(|n| !n.contains("no log found"));
    }
    let events = texts.iter().flat_map(|t| parse(t, since, tz)).collect();
    SourceResult { events, notes }
}

/// Parses a pacman timestamp. A trailing `+HHMM`/`-HHMM` offset is used if
/// present, otherwise the time is taken as local.
fn parse_time(s: &str, tz: &Tz) -> Option<i64> {
    let n = s.len();
    if n > 5 && s.is_char_boundary(n - 5) {
        let (clock, off) = s.split_at(n - 5);
        let sign = match off.as_bytes()[0] {
            b'+' => 1,
            b'-' => -1,
            _ => 0,
        };
        if sign != 0 && off[1..].bytes().all(|b| b.is_ascii_digit()) {
            let h: i64 = off[1..3].parse().ok()?;
            let m: i64 = off[3..].parse().ok()?;
            return Some(DateTime::parse(clock)?.to_epoch(sign * (h * HOUR + m * MINUTE)));
        }
    }
    Some(tz.to_epoch(DateTime::parse(s)?))
}

fn parse_line(line: &str, tz: &Tz) -> Option<Action> {
    let rest = line.strip_prefix('[')?;
    let (stamp, rest) = rest.split_once(']')?;
    let rest = rest.trim_start();
    let rest = match rest.strip_prefix("[ALPM]") {
        Some(r) => r.trim_start(),
        // A different tag ([PACMAN], [ALPM-SCRIPTLET]) means not an action.
        None if rest.starts_with('[') => return None,
        None => rest,
    };
    let (action, rest) = rest.split_once(' ')?;
    let verb = match action {
        "installed" => "installed",
        "upgraded" => "upgraded",
        "downgraded" => "downgraded",
        "reinstalled" => "reinstalled",
        "removed" => "removed",
        _ => return None,
    };
    let (name, version) = rest.split_once(" (")?;
    let version = version.strip_suffix(')')?;
    if name.contains(' ') {
        return None;
    }
    let time = parse_time(stamp, tz)?;
    Some(Action {
        time,
        verb,
        detail: format!("{name} {version}"),
    })
}

/// Parses log text, keeping actions at or after `since`, grouped per minute
/// like the dpkg source.
pub fn parse(text: &str, since: i64, tz: &Tz) -> Vec<Event> {
    group(text.lines().filter_map(|l| parse_line(l, tz)), since)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
[2024-05-01T09:12:00+0200] [PACMAN] Running 'pacman -Syu'
[2024-05-01T09:12:30+0200] [ALPM] transaction started
[2024-05-01T09:12:31+0200] [ALPM] upgraded mesa (1:24.1-1 -> 1:24.2-1)
[2024-05-01T09:12:32+0200] [ALPM] upgraded linux (6.12.4.arch1-1 -> 6.12.5.arch1-1)
[2024-05-01T09:12:33+0200] [ALPM-SCRIPTLET] ==> Building initramfs (installed foo (1))
[2024-05-01T09:12:40+0200] [ALPM] transaction completed
[2024-05-01T14:00:00+0200] [ALPM] installed htop (3.3.0-1)
[2024-05-01T14:00:05+0200] [ALPM] removed nano (7.2-1)
[2024-05-01T14:00:06+0200] [ALPM] downgraded vim (9.1-2 -> 9.1-1)
not a log line
";

    #[test]
    fn parses_and_groups() {
        let events = parse(SAMPLE, 0, &Tz::utc());
        let texts: Vec<&str> = events.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "upgraded  mesa 1:24.1-1 -> 1:24.2-1, linux 6.12.4.arch1-1 -> 6.12.5.arch1-1",
                "installed htop 3.3.0-1",
                "removed   nano 7.2-1",
                "downgraded vim 9.1-2 -> 9.1-1",
            ]
        );
        // 09:12:31 at +0200 is 07:12:31 UTC.
        assert_eq!(events[0].time, 1714547551);
    }

    #[test]
    fn old_formats_use_local_time() {
        let log = "\
[2019-01-01 12:00] [ALPM] installed foo (1.0-1)
[2012-01-01 12:00] upgraded bar (1 -> 2)
";
        let events = parse(log, 0, &Tz::fixed(HOUR));
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].text, "installed foo 1.0-1");
        assert_eq!(events[1].text, "upgraded  bar 1 -> 2");
        assert_eq!(
            events[0].time,
            DateTime::parse("2019-01-01 11:00").unwrap().to_epoch(0)
        );
    }

    #[test]
    fn negative_offset_and_since() {
        let t = parse_time("2024-05-01T09:00:00-0430", &Tz::utc()).unwrap();
        assert_eq!(t, DateTime::parse("2024-05-01 13:30").unwrap().to_epoch(0));
        // 2024-05-01 12:00 UTC: keeps only the afternoon actions.
        assert_eq!(parse(SAMPLE, 1714564800, &Tz::utc()).len(), 3);
    }

    #[test]
    fn missing_log_is_silent() {
        let root = crate::sources::TestRoot::new("pacman");
        let r = collect(&root.0, 0, &Tz::utc());
        assert!(r.events.is_empty() && r.notes.is_empty());
        root.file("/var/log/pacman.log", SAMPLE);
        assert_eq!(collect(&root.0, 0, &Tz::utc()).events.len(), 4);
    }
}
