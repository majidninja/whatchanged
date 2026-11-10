//! The timeline: a sorted list of change events from all sources.

use crate::tz::Tz;

/// One thing that changed on the machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// Seconds since the Unix epoch.
    pub time: i64,
    /// Short category shown in the second column: "pkg", "config", ...
    pub kind: &'static str,
    /// Human-readable description.
    pub text: String,
}

/// What a source reports back: the events it found, plus notes about
/// anything it could not read (missing file, no permission, ...).
#[derive(Debug, Default)]
pub struct SourceResult {
    pub events: Vec<Event>,
    pub notes: Vec<String>,
}

/// Formats events as plain text, one per line. A date header is printed
/// whenever the day changes, so multi-day timelines stay readable.
pub fn render(events: &[Event], tz: &Tz) -> String {
    let mut sorted: Vec<&Event> = events.iter().collect();
    sorted.sort_by_key(|e| e.time);
    let mut out = String::new();
    let mut last_day = None;
    for e in sorted {
        let dt = tz.local_time(e.time);
        let day = (dt.year, dt.month, dt.day);
        if last_day != Some(day) {
            out.push_str(&format!("{:04}-{:02}-{:02}\n", dt.year, dt.month, dt.day));
            last_day = Some(day);
        }
        out.push_str(&format!(
            "  {:02}:{:02}  {:<7} {}\n",
            dt.hour, dt.minute, e.kind, e.text
        ));
    }
    out
}

/// Events above which `summary` gives a line: a short timeline is its
/// own summary.
pub const SUMMARY_MIN: usize = 15;

/// One line counting the events by kind, most common first, e.g.
/// "23 changes: 12 pkg, 8 config, 3 service". `None` for short
/// timelines (`SUMMARY_MIN` events or fewer).
pub fn summary(events: &[Event]) -> Option<String> {
    if events.len() <= SUMMARY_MIN {
        return None;
    }
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for e in events {
        match counts.iter_mut().find(|(k, _)| *k == e.kind) {
            Some((_, n)) => *n += 1,
            None => counts.push((e.kind, 1)),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let parts: Vec<String> = counts.iter().map(|(k, n)| format!("{n} {k}")).collect();
    Some(format!("{} changes: {}", events.len(), parts.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_counts_by_kind() {
        let ev = |kind| Event {
            time: 0,
            kind,
            text: String::new(),
        };
        let mut events: Vec<Event> = (0..SUMMARY_MIN).map(|_| ev("pkg")).collect();
        assert_eq!(summary(&events), None);
        events.push(ev("service"));
        events.push(ev("config"));
        events.push(ev("config"));
        events.push(ev("cron"));
        assert_eq!(
            summary(&events).as_deref(),
            Some("19 changes: 15 pkg, 2 config, 1 cron, 1 service")
        );
    }

    #[test]
    fn renders_sorted_with_day_headers() {
        let ev = |time, text: &str| Event {
            time,
            kind: "pkg",
            text: text.into(),
        };
        // 2024-05-01 09:12 and 2024-05-02 14:40 UTC, given out of order.
        let events = vec![ev(1714660800, "b"), ev(1714554720, "a")];
        let out = render(&events, &Tz::utc());
        assert!(out.starts_with("2024-05-01\n  09:12  pkg     a\n2024-05-02\n"));
    }
}
