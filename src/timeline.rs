//! The timeline: a sorted list of change events from all sources.

use crate::time::DateTime;

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
pub fn render(events: &[Event], offset: i64) -> String {
    let mut sorted: Vec<&Event> = events.iter().collect();
    sorted.sort_by_key(|e| e.time);
    let mut out = String::new();
    let mut last_day = None;
    for e in sorted {
        let dt = DateTime::from_epoch(e.time, offset);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_sorted_with_day_headers() {
        let ev = |time, text: &str| Event {
            time,
            kind: "pkg",
            text: text.into(),
        };
        // 2024-05-01 09:12 and 2024-05-02 14:40 UTC, given out of order.
        let events = vec![ev(1714660800, "b"), ev(1714554720, "a")];
        let out = render(&events, 0);
        assert!(out.starts_with("2024-05-01\n  09:12  pkg     a\n2024-05-02\n"));
    }
}
