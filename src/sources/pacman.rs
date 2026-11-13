//! Package changes from the pacman log (Arch, Manjaro, EndeavourOS, ...).
//!
//! Lines look like:
//! `[2024-05-01T09:12:00+0200] [ALPM] upgraded mesa (1:24.1-1 -> 1:24.2-1)`
//! Older logs use local time without an offset, `[2019-01-01 12:00]`, and
//! very old ones have no `[ALPM]` tag. We keep package actions and skip
//! `[ALPM-SCRIPTLET]` output. The `[PACMAN] Running 'pacman -Syu'` lines
//! say which command made a change; it is added after the change, like
//! `upgraded  mesa 24.1 -> 24.2 (pacman -Syu)`.

use std::path::Path;

use crate::sources::pkg::{Action, Run, annotate, group, shorten};
use crate::sources::read_logs;
use crate::timeline::{Event, SourceResult};
use crate::tz::Tz;

pub const LOGS: &[&str] = &["/var/log/pacman.log"];

pub fn collect(root: &Path, since: i64, tz: &Tz) -> SourceResult {
    let (texts, notes) = read_logs("pacman", root, LOGS, since);
    let mut events = Vec::new();
    for t in &texts {
        let mut ev = parse(t, since, tz);
        annotate(&mut ev, &runs(t, tz));
        events.extend(ev);
    }
    SourceResult { events, notes }
}

/// Splits the log into runs, one per `[PACMAN] Running '...'` line:
/// each lasts until the end of its transaction (`transaction completed`,
/// `failed` or `interrupted`) or the next command. pacman doesn't log who
/// ran it. Front ends that use libalpm directly (pamac, PackageKit) log no
/// command line, so their changes get none.
fn runs(text: &str, tz: &Tz) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    let mut open = false;
    for line in text.lines() {
        let Some((stamp, rest)) = line.strip_prefix('[').and_then(|r| r.split_once(']')) else {
            continue;
        };
        let Some(time) = tz.parse_stamp(stamp) else {
            continue;
        };
        let rest = rest.trim_start();
        if let Some(cmd) = rest
            .strip_prefix("[PACMAN] Running '")
            .and_then(|c| c.strip_suffix('\''))
        {
            if open && let Some(prev) = runs.last_mut() {
                prev.end = time - 1;
            }
            runs.push(Run {
                start: time,
                end: time + 3600,
                command: shorten(cmd),
                user: None,
            });
            open = true;
        } else if open
            && rest.starts_with("[ALPM] transaction ")
            && !rest.ends_with("transaction started")
        {
            if let Some(r) = runs.last_mut() {
                r.end = time;
            }
            open = false;
        }
    }
    runs
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
    let time = tz.parse_stamp(stamp)?;
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
    use crate::time::{DateTime, HOUR};

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
        let t = Tz::utc().parse_stamp("2024-05-01T09:00:00-0430").unwrap();
        assert_eq!(t, DateTime::parse("2024-05-01 13:30").unwrap().to_epoch(0));
        // 2024-05-01 12:00 UTC: keeps only the afternoon actions.
        assert_eq!(parse(SAMPLE, 1714564800, &Tz::utc()).len(), 3);
    }

    #[test]
    fn adds_the_command_that_made_the_change() {
        let root = crate::sources::TestRoot::new("pacman-runs");
        root.file("/var/log/pacman.log", SAMPLE);
        let r = collect(&root.0, 0, &Tz::utc());
        let texts: Vec<&str> = r.events.iter().map(|e| e.text.as_str()).collect();
        // The afternoon actions have no Running line (e.g. pamac): no command.
        assert_eq!(
            texts,
            [
                "upgraded  mesa 1:24.1-1 -> 1:24.2-1, linux 6.12.4.arch1-1 -> 6.12.5.arch1-1 (pacman -Syu)",
                "installed htop 3.3.0-1",
                "removed   nano 7.2-1",
                "downgraded vim 9.1-2 -> 9.1-1",
            ]
        );
    }

    #[test]
    fn runs_end_at_transaction_end_or_next_command() {
        let log = "\
[2024-05-01T09:00:00+0000] [PACMAN] Running 'pacman -Syu'
[2024-05-01T09:00:10+0000] [PACMAN] Running 'yay -S --needed foo'
[2024-05-01T09:00:20+0000] [ALPM] transaction started
[2024-05-01T09:00:30+0000] [ALPM] transaction failed
[2024-05-01T09:05:00+0000] [PACMAN] Running 'pacman -S bar'
";
        let r = runs(log, &Tz::utc());
        let spans: Vec<(i64, &str)> = r
            .iter()
            .map(|r| (r.end - r.start, r.command.as_str()))
            .collect();
        // The last run never finished: it is taken to last an hour.
        assert_eq!(
            spans,
            [
                (9, "pacman -Syu"),
                (20, "yay -S --needed foo"),
                (3600, "pacman -S bar")
            ]
        );
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
