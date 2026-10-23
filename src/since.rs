//! Parsing the `--since` argument into an epoch timestamp.

use crate::time::{DAY, DateTime, HOUR, MINUTE};

/// Accepted forms:
/// - `today`, `yesterday` (start of that local day)
/// - `30m`, `2h`, `3d`, `1w` (that long before now)
/// - `2024-05-01`, `2024-05-01 09:00`, `2024-05-01T09:00:00` (local time)
pub fn parse_since(s: &str, now: i64, offset: i64) -> Result<i64, String> {
    let s = s.trim();
    let start_of_today = || {
        let local = now + offset;
        local - local.rem_euclid(DAY) - offset
    };
    match s {
        "today" => return Ok(start_of_today()),
        "yesterday" => return Ok(start_of_today() - DAY),
        _ => {}
    }
    if let Some(unit) = s.chars().last().filter(|c| c.is_ascii_alphabetic()) {
        let num = &s[..s.len() - 1];
        if let Ok(n) = num.parse::<i64>() {
            let secs = match unit {
                'm' => MINUTE,
                'h' => HOUR,
                'd' => DAY,
                'w' => 7 * DAY,
                _ => return Err(format!("unknown unit '{unit}' in '{s}' (use m, h, d or w)")),
            };
            return Ok(now - n * secs);
        }
    }
    DateTime::parse(s)
        .map(|dt| dt.to_epoch(offset))
        .ok_or_else(|| {
            format!("can't understand --since '{s}' (try: yesterday, 2h, 3d, 2024-05-01)")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2024-05-01 09:12:33 UTC
    const NOW: i64 = 1714554753;
    const MIDNIGHT: i64 = 1714521600;

    #[test]
    fn relative() {
        assert_eq!(parse_since("2h", NOW, 0), Ok(NOW - 2 * HOUR));
        assert_eq!(parse_since("3d", NOW, 0), Ok(NOW - 3 * DAY));
        assert_eq!(parse_since("15m", NOW, 0), Ok(NOW - 15 * MINUTE));
        assert!(parse_since("3y", NOW, 0).is_err());
    }

    #[test]
    fn named_days() {
        assert_eq!(parse_since("today", NOW, 0), Ok(MIDNIGHT));
        assert_eq!(parse_since("yesterday", NOW, 0), Ok(MIDNIGHT - DAY));
        // At UTC+10 it is already 19:12 local, same local day; midnight is 10h earlier in UTC.
        assert_eq!(
            parse_since("today", NOW, 10 * HOUR),
            Ok(MIDNIGHT - 10 * HOUR)
        );
    }

    #[test]
    fn dates() {
        assert_eq!(parse_since("2024-05-01", NOW, 0), Ok(MIDNIGHT));
        assert_eq!(parse_since("2024-05-01 01:00", NOW, 0), Ok(MIDNIGHT + HOUR));
        assert!(parse_since("last tuesday", NOW, 0).is_err());
    }
}
