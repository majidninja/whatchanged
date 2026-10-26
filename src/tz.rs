//! Local time zone support without a crate.
//!
//! We read the compiled zone file (TZif, see `man 5 tzfile`) that the system
//! uses: `$TZ` if set, else `/etc/localtime`. Times after the file's last
//! transition follow the POSIX rule string stored at the end of the file
//! (e.g. `CET-1CEST,M3.5.0,M10.5.0/3`), which `$TZ` may also contain directly.

use std::path::{Path, PathBuf};

use crate::time::{DAY, DateTime, HOUR, MINUTE, days_from_civil};

/// A time zone: offsets east of UTC, in seconds, that change over time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tz {
    /// (UTC time of change, offset from then on), sorted by time.
    transitions: Vec<(i64, i64)>,
    /// Offset before the first transition.
    initial: i64,
    /// Rule for times after the last transition (or for all times, if none).
    rule: Option<Rule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Rule {
    Fixed(i64),
    Dst {
        std: i64,
        dst: i64,
        start: (Day, i64),
        end: (Day, i64),
    },
}

/// The day of the year a DST change happens, in POSIX TZ terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Day {
    /// `Jn`: day 1..=365, February 29 is never counted.
    Julian(i64),
    /// `n`: day 0..=365, counting February 29 in leap years.
    Zero(i64),
    /// `Mm.w.d`: weekday d (0 = Sunday) of week w (5 = last) of month m.
    Month(u32, u32, i64),
}

impl Tz {
    pub fn utc() -> Self {
        Tz::fixed(0)
    }

    pub fn fixed(offset: i64) -> Self {
        Tz {
            transitions: Vec::new(),
            initial: offset,
            rule: Some(Rule::Fixed(offset)),
        }
    }

    /// The zone of the system at `root` (normally `/`). If it can't be
    /// determined, returns UTC and a note saying why.
    ///
    /// `$TZ` only applies to the live system: with another root we want that
    /// system's zone, from `<root>/etc/localtime`.
    pub fn local(root: &Path) -> (Self, Option<String>) {
        match std::env::var("TZ") {
            Ok(tz) if root == Path::new("/") && !tz.is_empty() => Tz::from_tz_var(&tz),
            _ => {
                let path = resolve_under(root, "/etc/localtime");
                match std::fs::read(&path) {
                    Ok(data) => match Tz::from_tzif(&data) {
                        Some(tz) => (tz, None),
                        None => (
                            Tz::utc(),
                            Some(note(&format!("{} is not a zone file", path.display()))),
                        ),
                    },
                    Err(e) => (Tz::utc(), Some(note(&format!("{}: {e}", path.display())))),
                }
            }
        }
    }

    fn from_tz_var(tz: &str) -> (Self, Option<String>) {
        let name = tz.strip_prefix(':').unwrap_or(tz);
        let path = if name.starts_with('/') {
            name.to_string()
        } else {
            format!("/usr/share/zoneinfo/{name}")
        };
        if !name.contains("..")
            && let Some(tz) = std::fs::read(&path).ok().and_then(|d| Tz::from_tzif(&d))
        {
            return (tz, None);
        }
        match parse_posix(name) {
            Some(rule) => (Tz::from_rule(rule), None),
            None => (Tz::utc(), Some(note(&format!("unknown TZ '{tz}'")))),
        }
    }

    fn from_rule(rule: Rule) -> Self {
        let initial = match rule {
            Rule::Fixed(o) | Rule::Dst { std: o, .. } => o,
        };
        Tz {
            transitions: Vec::new(),
            initial,
            rule: Some(rule),
        }
    }

    /// Parses TZif data (versions 1 to 4). Uses the 64-bit section when present.
    pub fn from_tzif(data: &[u8]) -> Option<Self> {
        let (v1, rest) = parse_block(data, 4)?;
        if data[4] == 0 {
            return Some(v1);
        }
        let (mut tz, footer) = parse_block(rest, 8)?;
        let footer = footer.strip_prefix(b"\n")?;
        let end = footer.iter().position(|&b| b == b'\n')?;
        let footer = std::str::from_utf8(&footer[..end]).ok()?;
        tz.rule = if footer.is_empty() {
            None
        } else {
            Some(parse_posix(footer)?)
        };
        Some(tz)
    }

    /// Offset east of UTC, in seconds, in effect at UTC time `ts`.
    pub fn offset_at(&self, ts: i64) -> i64 {
        let after_last = self.transitions.last().is_none_or(|&(t, _)| ts >= t);
        if after_last && let Some(rule) = &self.rule {
            return rule.offset_at(ts);
        }
        match self.transitions.partition_point(|&(t, _)| t <= ts) {
            0 => self.initial,
            i => self.transitions[i - 1].1,
        }
    }

    /// Local wall-clock time at UTC time `ts`.
    pub fn local_time(&self, ts: i64) -> DateTime {
        DateTime::from_epoch(ts, self.offset_at(ts))
    }

    /// UTC time for a local wall-clock time. Times that don't exist or happen
    /// twice (around DST changes) get a reasonable nearby answer.
    pub fn to_epoch(&self, dt: DateTime) -> i64 {
        let local = dt.to_epoch(0);
        let guess = local - self.offset_at(local);
        local - self.offset_at(guess)
    }
}

fn note(why: &str) -> String {
    format!("time zone: {why}; showing times in UTC")
}

fn be(bytes: &[u8]) -> i64 {
    let mut v: i64 = if bytes[0] & 0x80 != 0 { -1 } else { 0 };
    for &b in bytes {
        v = (v << 8) | b as i64;
    }
    v
}

/// Parses one header + data block with `size`-byte times; returns the rest.
fn parse_block(data: &[u8], size: usize) -> Option<(Tz, &[u8])> {
    if data.len() < 44 || &data[..4] != b"TZif" {
        return None;
    }
    let count = |i: usize| {
        be(&[
            0,
            data[20 + 4 * i],
            data[21 + 4 * i],
            data[22 + 4 * i],
            data[23 + 4 * i],
        ]) as usize
    };
    let (isut, isstd, leap, time, typ, chars) =
        (count(0), count(1), count(2), count(3), count(4), count(5));
    let body = &data[44..];
    let len = time * size + time + typ * 6 + chars + leap * (size + 4) + isstd + isut;
    if body.len() < len || typ == 0 {
        return None;
    }
    let types: Vec<i64> = (0..typ)
        .map(|i| be(&body[time * (size + 1) + i * 6..][..4]))
        .collect();
    let mut transitions = Vec::with_capacity(time);
    for i in 0..time {
        let t = be(&body[i * size..][..size]);
        let idx = body[time * size + i] as usize;
        transitions.push((t, *types.get(idx)?));
    }
    let tz = Tz {
        transitions,
        initial: types[0],
        rule: None,
    };
    Some((tz, &body[len..]))
}

impl Rule {
    fn offset_at(&self, ts: i64) -> i64 {
        match *self {
            Rule::Fixed(o) => o,
            Rule::Dst {
                std,
                dst,
                start,
                end,
            } => {
                let year = DateTime::from_epoch(ts, std).year;
                // Start happens in standard time, end in daylight time.
                let s = day_in_year(year, start.0) * DAY + start.1 - std;
                let e = day_in_year(year, end.0) * DAY + end.1 - dst;
                let in_dst = if s < e {
                    s <= ts && ts < e
                } else {
                    ts < e || ts >= s
                };
                if in_dst { dst } else { std }
            }
        }
    }
}

/// Days since the epoch of the rule day `d` in `year`.
fn day_in_year(year: i64, d: Day) -> i64 {
    let jan1 = days_from_civil(year, 1, 1);
    let leap = days_from_civil(year, 3, 1) - days_from_civil(year, 2, 28) == 2;
    match d {
        Day::Julian(n) => jan1 + n - 1 + i64::from(leap && n >= 60),
        Day::Zero(n) => jan1 + n,
        Day::Month(m, w, wd) => {
            let first = days_from_civil(year, m, 1);
            // 1970-01-01 was a Thursday (4).
            let mut day = first + (wd - (first + 4)).rem_euclid(7) + (w as i64 - 1) * 7;
            let next_month = if m == 12 {
                days_from_civil(year + 1, 1, 1)
            } else {
                days_from_civil(year, m + 1, 1)
            };
            while day >= next_month {
                day -= 7;
            }
            day
        }
    }
}

/// Follows symlinks for `path` while staying below `root`. `/etc/localtime`
/// is usually an absolute link like `/usr/share/zoneinfo/Europe/Berlin`;
/// on a mounted disk that must point into the disk, not into the host.
fn resolve_under(root: &Path, path: &str) -> PathBuf {
    let mut full = crate::sources::under(root, path);
    for _ in 0..8 {
        let Ok(target) = std::fs::read_link(&full) else {
            break;
        };
        full = match target.strip_prefix("/") {
            Ok(rel) => root.join(rel),
            Err(_) => full.parent().unwrap_or(root).join(target),
        };
    }
    full
}

/// Parses a POSIX TZ string such as `UTC0`, `<+03>-3` or
/// `CET-1CEST,M3.5.0,M10.5.0/3`.
fn parse_posix(s: &str) -> Option<Rule> {
    let mut p = Parser(s);
    p.name()?;
    // POSIX offsets count hours *west* of UTC, so flip the sign.
    let std = -p.time()?;
    if p.0.is_empty() {
        return Some(Rule::Fixed(std));
    }
    p.name()?;
    let dst = if p.0.starts_with(',') {
        std + HOUR
    } else {
        -p.time()?
    };
    // Without a rule, POSIX says the default is implementation-defined; use US rules.
    if p.0.is_empty() {
        p = Parser(",M3.2.0,M11.1.0");
    }
    let mut rule_part = || -> Option<(Day, i64)> {
        p.eat(',')?;
        let day = p.day()?;
        let time = if p.eat('/').is_some() {
            p.time()?
        } else {
            2 * HOUR
        };
        Some((day, time))
    };
    let start = rule_part()?;
    let end = rule_part()?;
    p.0.is_empty().then_some(Rule::Dst {
        std,
        dst,
        start,
        end,
    })
}

struct Parser<'a>(&'a str);

impl Parser<'_> {
    fn eat(&mut self, c: char) -> Option<()> {
        self.0 = self.0.strip_prefix(c)?;
        Some(())
    }

    fn take_while(&mut self, f: impl Fn(char) -> bool) -> &str {
        let end = self.0.find(|c| !f(c)).unwrap_or(self.0.len());
        let (head, tail) = self.0.split_at(end);
        self.0 = tail;
        head
    }

    fn name(&mut self) -> Option<()> {
        let n = if self.eat('<').is_some() {
            let n = self.take_while(|c| c != '>').len();
            self.eat('>')?;
            n
        } else {
            self.take_while(|c| c.is_ascii_alphabetic()).len()
        };
        (n >= 3).then_some(())
    }

    fn num(&mut self) -> Option<i64> {
        self.take_while(|c| c.is_ascii_digit()).parse().ok()
    }

    /// `[+-]hh[:mm[:ss]]` as seconds.
    fn time(&mut self) -> Option<i64> {
        let sign = if self.eat('-').is_some() {
            -1
        } else {
            self.eat('+');
            1
        };
        let mut secs = self.num()? * HOUR;
        if self.eat(':').is_some() {
            secs += self.num()? * MINUTE;
            if self.eat(':').is_some() {
                secs += self.num()?;
            }
        }
        Some(sign * secs)
    }

    fn day(&mut self) -> Option<Day> {
        if self.eat('J').is_some() {
            let n = self.num()?;
            return (1..=365).contains(&n).then_some(Day::Julian(n));
        }
        if self.eat('M').is_some() {
            let m = self.num()?;
            self.eat('.')?;
            let w = self.num()?;
            self.eat('.')?;
            let d = self.num()?;
            let ok = (1..=12).contains(&m) && (1..=5).contains(&w) && (0..=6).contains(&d);
            return ok.then_some(Day::Month(m as u32, w as u32, d));
        }
        let n = self.num()?;
        (0..=365).contains(&n).then_some(Day::Zero(n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> i64 {
        DateTime::parse(s).unwrap().to_epoch(0)
    }

    #[test]
    fn posix_fixed() {
        assert_eq!(parse_posix("UTC0"), Some(Rule::Fixed(0)));
        assert_eq!(
            parse_posix("<+0530>-5:30"),
            Some(Rule::Fixed(5 * HOUR + 30 * MINUTE))
        );
        assert_eq!(parse_posix("EST5"), Some(Rule::Fixed(-5 * HOUR)));
        assert_eq!(parse_posix("bogus"), None);
    }

    #[test]
    fn posix_europe() {
        let tz = Tz::from_rule(parse_posix("CET-1CEST,M3.5.0,M10.5.0/3").unwrap());
        // 2024: DST from Sun 31 March 01:00 UTC to Sun 27 October 01:00 UTC.
        assert_eq!(tz.offset_at(at("2024-03-31 00:59")), HOUR);
        assert_eq!(tz.offset_at(at("2024-03-31 01:00")), 2 * HOUR);
        assert_eq!(tz.offset_at(at("2024-10-27 00:59")), 2 * HOUR);
        assert_eq!(tz.offset_at(at("2024-10-27 01:00")), HOUR);
        let local = DateTime::parse("2024-07-01 12:00").unwrap();
        assert_eq!(tz.to_epoch(local), at("2024-07-01 10:00"));
        assert_eq!(tz.local_time(at("2024-07-01 10:00")), local);
    }

    #[test]
    fn posix_southern() {
        // Sydney: DST from first Sunday in October to first Sunday in April.
        let tz = Tz::from_rule(parse_posix("AEST-10AEDT,M10.1.0,M4.1.0/3").unwrap());
        assert_eq!(tz.offset_at(at("2024-01-15 00:00")), 11 * HOUR);
        assert_eq!(tz.offset_at(at("2024-07-15 00:00")), 10 * HOUR);
    }

    /// Builds a minimal TZif v2 file: two types, the given transitions, a footer.
    fn tzif(transitions: &[(i64, u8)], footer: &str) -> Vec<u8> {
        let types: [(i32, u8); 2] = [(3600, 0), (7200, 1)];
        let block = |size: usize| {
            let mut b = b"TZif2".to_vec();
            b.extend([0; 15]);
            for n in [0, 0, 0, transitions.len(), types.len(), 4] {
                b.extend((n as u32).to_be_bytes());
            }
            for &(t, _) in transitions {
                b.extend(&t.to_be_bytes()[8 - size..]);
            }
            b.extend(transitions.iter().map(|&(_, i)| i));
            for (off, dst) in types {
                b.extend(off.to_be_bytes());
                b.extend([dst, 0]);
            }
            b.extend(b"CET\0");
            b
        };
        let mut data = block(4);
        data.extend(block(8));
        data.extend(format!("\n{footer}\n").as_bytes());
        data
    }

    #[test]
    fn tzif_transitions_then_footer() {
        let summer = at("2000-03-26 01:00");
        let winter = at("2000-10-29 01:00");
        let data = tzif(&[(summer, 1), (winter, 0)], "CET-1CEST,M3.5.0,M10.5.0/3");
        let tz = Tz::from_tzif(&data).unwrap();
        assert_eq!(tz.offset_at(summer - 1), HOUR);
        assert_eq!(tz.offset_at(summer), 2 * HOUR);
        assert_eq!(tz.offset_at(winter), HOUR);
        // Past the table: the footer rule applies.
        assert_eq!(tz.offset_at(at("2030-07-01 00:00")), 2 * HOUR);
        assert_eq!(tz.offset_at(at("2030-12-01 00:00")), HOUR);
    }

    #[test]
    fn tzif_rejects_garbage() {
        assert_eq!(Tz::from_tzif(b"not a zone file"), None);
        let mut data = tzif(&[], "CET-1");
        data.truncate(60);
        assert_eq!(Tz::from_tzif(&data), None);
    }

    #[test]
    fn local_under_root_follows_absolute_link_inside_root() {
        let root = crate::sources::TestRoot::new("tz-root");
        root.file("/usr/share/zoneinfo/Test/Zone", "");
        root.file("/etc/placeholder", "");
        let zone = crate::sources::under(&root.0, "/usr/share/zoneinfo/Test/Zone");
        std::fs::write(&zone, tzif(&[], "CET-1")).unwrap();
        std::os::unix::fs::symlink(
            "/usr/share/zoneinfo/Test/Zone",
            crate::sources::under(&root.0, "/etc/localtime"),
        )
        .unwrap();
        let (tz, note) = Tz::local(&root.0);
        assert_eq!(note, None);
        assert_eq!(tz.offset_at(0), HOUR);
    }

    #[test]
    fn local_under_root_without_zone_is_utc_with_note() {
        let root = crate::sources::TestRoot::new("tz-none");
        let (tz, note) = Tz::local(&root.0);
        assert_eq!(tz, Tz::utc());
        assert!(note.unwrap().contains("etc/localtime"));
    }

    /// Checks against real tzdata. Not run by default because it depends on
    /// the machine: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn system_zoneinfo() {
        let read = |name| {
            Tz::from_tzif(&std::fs::read(format!("/usr/share/zoneinfo/{name}")).unwrap()).unwrap()
        };
        let berlin = read("Europe/Berlin");
        assert_eq!(berlin.offset_at(at("2024-01-15 12:00")), HOUR);
        assert_eq!(berlin.offset_at(at("2024-07-15 12:00")), 2 * HOUR);
        assert_eq!(berlin.offset_at(at("2045-07-15 12:00")), 2 * HOUR);
        let kolkata = read("Asia/Kolkata");
        assert_eq!(
            kolkata.offset_at(at("2024-07-15 12:00")),
            5 * HOUR + 30 * MINUTE
        );
        let ny = read("America/New_York");
        assert_eq!(ny.offset_at(at("2024-01-15 12:00")), -5 * HOUR);
        assert_eq!(ny.offset_at(at("2024-07-15 12:00")), -4 * HOUR);
    }
}
