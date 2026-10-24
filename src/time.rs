//! Minimal calendar math on top of std, so we don't need a date crate.
//!
//! Timestamps are seconds since the Unix epoch (UTC). Wall-clock times read
//! from logs are converted to UTC by the time zone code in `tz.rs`.

use std::time::{SystemTime, UNIX_EPOCH};

pub const MINUTE: i64 = 60;
pub const HOUR: i64 = 60 * MINUTE;
pub const DAY: i64 = 24 * HOUR;

/// A wall-clock date and time without a time zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DateTime {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
/// Algorithm from Howard Hinnant's "chrono-compatible low-level date algorithms".
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = month as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Inverse of `days_from_civil`.
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        _ => 0,
    }
}

impl DateTime {
    /// Builds a DateTime, rejecting impossible values like Feb 30.
    pub fn new(
        year: i64,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
    ) -> Option<Self> {
        if !(1..=12).contains(&month)
            || day == 0
            || day > days_in_month(year, month)
            || hour > 23
            || minute > 59
            || second > 60
        {
            return None;
        }
        Some(DateTime {
            year,
            month,
            day,
            hour,
            minute,
            second,
        })
    }

    /// Seconds since the epoch, treating this wall-clock time as UTC+`offset`.
    pub fn to_epoch(self, offset: i64) -> i64 {
        days_from_civil(self.year, self.month, self.day) * DAY
            + self.hour as i64 * HOUR
            + self.minute as i64 * MINUTE
            + self.second as i64
            - offset
    }

    pub fn from_epoch(ts: i64, offset: i64) -> Self {
        let local = ts + offset;
        let (year, month, day) = civil_from_days(local.div_euclid(DAY));
        let secs = local.rem_euclid(DAY);
        DateTime {
            year,
            month,
            day,
            hour: (secs / HOUR) as u32,
            minute: (secs % HOUR / MINUTE) as u32,
            second: (secs % MINUTE) as u32,
        }
    }

    /// Parses "YYYY-MM-DD", "YYYY-MM-DD HH:MM", "YYYY-MM-DD HH:MM:SS"
    /// (a 'T' is accepted instead of the space).
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let (date, time) = match s.split_once([' ', 'T']) {
            Some((d, t)) => (d, Some(t)),
            None => (s, None),
        };
        let mut d = date.split('-');
        let year = d.next()?.parse().ok()?;
        let month = d.next()?.parse().ok()?;
        let day = d.next()?.parse().ok()?;
        if d.next().is_some() {
            return None;
        }
        let (mut hour, mut minute, mut second) = (0, 0, 0);
        if let Some(t) = time {
            let mut t = t.split(':');
            hour = t.next()?.parse().ok()?;
            minute = t.next()?.parse().ok()?;
            if let Some(sec) = t.next() {
                second = sec.parse().ok()?;
            }
            if t.next().is_some() {
                return None;
            }
        }
        DateTime::new(year, month, day, hour, minute, second)
    }
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_roundtrip() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11017);
        for days in [-1000, 0, 11017, 19000, 20000, 30000] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
    }

    #[test]
    fn parse_and_convert() {
        let dt = DateTime::parse("2024-05-01 09:12:33").unwrap();
        assert_eq!(dt.to_epoch(0), 1714554753);
        assert_eq!(DateTime::from_epoch(1714554753, 0), dt);
        assert_eq!(dt.to_epoch(2 * HOUR), 1714554753 - 7200);
        assert!(DateTime::parse("2024-02-30").is_none());
        assert!(DateTime::parse("2024-02-29T10:00").is_some());
        assert!(DateTime::parse("yesterday").is_none());
    }
}
