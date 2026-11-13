//! Boots and shutdowns, from the login records.
//!
//! "It worked yesterday" is often "it worked before the reboot", so the
//! timeline shows when the machine came up and with which kernel:
//! `booted 6.12.5-amd64`, and `shut down` when it went down cleanly.
//!
//! Two places hold this, depending on the distro's age:
//! - wtmpdb's SQLite file `/var/lib/wtmpdb/wtmp.db` (Debian 13, Fedora 43
//!   and newer): one row per boot with its start, its end and the kernel.
//! - the classic binary `/var/log/wtmp` (and `wtmp.1`): `BOOT_TIME`
//!   records, and a `shutdown` run-level record on the way down.
//!
//! If neither has the current boot (no record yet, or a container), the
//! boot time in `/proc/stat` is used for the running system.

use std::io::ErrorKind;
use std::path::Path;

use crate::sources::kernel::{boot_time, read_trimmed};
use crate::sources::under;
use crate::sqlite::{self, Value};
use crate::timeline::{Event, SourceResult};

/// wtmpdb's database; the second path is where the first usually links to.
const WTMPDB: &[&str] = &["/var/lib/wtmpdb/wtmp.db", "/var/log/wtmp.db"];

/// The classic login record, oldest first.
const WTMP: &[&str] = &["/var/log/wtmp.1", "/var/log/wtmp"];

/// Largest login record file we read. They are normally well under 1 MB.
const MAX_SIZE: u64 = 64 << 20;

pub fn collect(root: &Path, since: i64) -> SourceResult {
    let mut result = SourceResult::default();
    // (time, kernel) of each boot and the time of each shutdown, any age.
    let mut boots: Vec<(i64, String)> = Vec::new();
    let mut downs: Vec<i64> = Vec::new();

    for path in WTMPDB {
        if let Some(data) = read(root, path, &mut result.notes) {
            match sqlite::read_table(&data, "wtmp") {
                Ok(rows) => wtmpdb_rows(&rows, &mut boots, &mut downs),
                Err(e) => result.notes.push(format!(
                    "boot: can't read {}: {e}",
                    under(root, path).display()
                )),
            }
            break; // the second path is the same file
        }
    }
    for path in WTMP {
        if let Some(data) = read(root, path, &mut result.notes) {
            wtmp_records(&data, &mut boots, &mut downs);
        }
    }
    // The running system's own boot, when the records don't have it.
    if let Some(now_boot) = read_trimmed(root, "/proc/stat").and_then(|s| boot_time(&s))
        && !boots.iter().any(|(t, _)| (t - now_boot).abs() < NEAR)
    {
        let kernel = read_trimmed(root, "/proc/sys/kernel/osrelease").unwrap_or_default();
        boots.push((now_boot, kernel));
    }

    boots.sort();
    // Both record files may hold the same boot.
    boots.dedup_by(|b, a| (b.0 - a.0).abs() < NEAR);
    downs.sort();
    downs.dedup_by(|b, a| (*b - *a).abs() < NEAR);
    for (time, kernel) in boots {
        if time >= since {
            result.events.push(Event {
                time,
                kind: "boot",
                text: format!("booted {kernel}").trim_end().to_string(),
            });
        }
    }
    for time in downs {
        if time >= since {
            result.events.push(Event {
                time,
                kind: "boot",
                text: "shut down".to_string(),
            });
        }
    }
    result
}

/// Two records this close together (seconds) are the same boot or
/// shutdown, written by two tools.
const NEAR: i64 = 120;

/// Reads a record file. Missing is normal (the distro uses the other
/// kind); anything else is a note.
fn read(root: &Path, path: &str, notes: &mut Vec<String>) -> Option<Vec<u8>> {
    let full = under(root, path);
    match std::fs::metadata(&full) {
        Ok(m) if m.len() > MAX_SIZE => {
            notes.push(format!(
                "boot: {} is too big to read ({} MB)",
                full.display(),
                m.len() >> 20
            ));
            return None;
        }
        Ok(m) if !m.is_file() => return None,
        _ => {}
    }
    match std::fs::read(&full) {
        Ok(data) => Some(data),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) if e.kind() == ErrorKind::PermissionDenied => {
            notes.push(format!("boot: no permission to read {}", full.display()));
            None
        }
        Err(e) => {
            notes.push(format!("boot: can't read {}: {e}", full.display()));
            None
        }
    }
}

/// wtmpdb rows are (ID, Type, User, Login, Logout, TTY, RemoteHost,
/// Service) with times in microseconds. Type 1 is a boot: Login is when
/// it came up, Logout when it went down, RemoteHost the kernel version.
fn wtmpdb_rows(rows: &[Vec<Value>], boots: &mut Vec<(i64, String)>, downs: &mut Vec<i64>) {
    for row in rows {
        let [_, Value::Int(1), _, Value::Int(login), logout, _, host, ..] = row.as_slice() else {
            continue;
        };
        let kernel = match host {
            Value::Text(k) => k.clone(),
            _ => String::new(),
        };
        boots.push((login / 1_000_000, kernel));
        if let Value::Int(logout) = logout {
            downs.push(logout / 1_000_000);
        }
    }
}

/// Size of one record in the classic wtmp file (glibc's `struct utmp`,
/// the same on x86_64 and aarch64).
const RECORD: usize = 384;

/// Classic wtmp: type 2 (`BOOT_TIME`) has the kernel version in the host
/// field; type 1 (`RUN_LVL`) with user `shutdown` is written on the way
/// down.
fn wtmp_records(data: &[u8], boots: &mut Vec<(i64, String)>, downs: &mut Vec<i64>) {
    let text = |bytes: &[u8]| {
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        String::from_utf8_lossy(&bytes[..end]).into_owned()
    };
    for r in data.as_chunks::<RECORD>().0 {
        let kind = i16::from_le_bytes([r[0], r[1]]);
        // Seconds are stored as 32 bits, unsigned on current systems.
        let time = i64::from(u32::from_le_bytes([r[340], r[341], r[342], r[343]]));
        if time == 0 {
            continue;
        }
        match kind {
            2 => boots.push((time, text(&r[76..332]))),
            1 if text(&r[44..76]) == "shutdown" => downs.push(time),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::TestRoot;

    /// tests/data/wtmp.db: booted 6.12.4-test 2024-05-01 08:00:05 UTC,
    /// shut down 2024-05-02 22:10:00, booted 6.12.5-test 22:11:30; and
    /// one login row that must be ignored.
    const DB: &[u8] = include_bytes!("../../tests/data/wtmp.db");
    const BOOT1: i64 = 1714550405;
    const DOWN1: i64 = 1714687800;
    const BOOT2: i64 = 1714687890;

    /// One classic wtmp record.
    fn record(kind: i16, user: &str, host: &str, time: u32) -> Vec<u8> {
        let mut r = vec![0u8; RECORD];
        r[..2].copy_from_slice(&kind.to_le_bytes());
        r[44..44 + user.len()].copy_from_slice(user.as_bytes());
        r[76..76 + host.len()].copy_from_slice(host.as_bytes());
        r[340..344].copy_from_slice(&time.to_le_bytes());
        r
    }

    fn lines(r: &SourceResult) -> Vec<(i64, &str)> {
        r.events.iter().map(|e| (e.time, e.text.as_str())).collect()
    }

    #[test]
    fn reads_boots_and_shutdowns_from_wtmpdb() {
        let root = TestRoot::new("boot-db");
        root.bytes("/var/lib/wtmpdb/wtmp.db", DB);
        let r = collect(&root.0, 0);
        assert!(r.notes.is_empty(), "{:?}", r.notes);
        assert!(r.events.iter().all(|e| e.kind == "boot"));
        assert_eq!(
            lines(&r),
            [
                (BOOT1, "booted 6.12.4-test"),
                (BOOT2, "booted 6.12.5-test"),
                (DOWN1, "shut down"),
            ]
        );
        // --since cuts the first boot off.
        assert_eq!(lines(&collect(&root.0, DOWN1)).len(), 2);
        // The other usual location works too.
        let root = TestRoot::new("boot-db-log");
        root.bytes("/var/log/wtmp.db", DB);
        assert_eq!(collect(&root.0, 0).events.len(), 3);
    }

    #[test]
    fn reads_boots_and_shutdowns_from_classic_wtmp() {
        let root = TestRoot::new("boot-wtmp");
        let mut old = record(2, "reboot", "6.1.0-old", 1_700_000_000);
        old.extend(record(7, "alice", "10.0.0.1", 1_700_000_100)); // a login
        let mut new = record(1, "shutdown", "6.1.0-old", 1_700_500_000);
        new.extend(record(1, "runlevel", "6.1.0-new", 1_700_500_090)); // not a shutdown
        new.extend(record(2, "reboot", "6.1.0-new", 1_700_500_060));
        new.extend([1, 2, 3]); // a torn last record
        root.bytes("/var/log/wtmp.1", &old);
        root.bytes("/var/log/wtmp", &new);
        let r = collect(&root.0, 0);
        assert!(r.notes.is_empty(), "{:?}", r.notes);
        assert_eq!(
            lines(&r),
            [
                (1_700_000_000, "booted 6.1.0-old"),
                (1_700_500_060, "booted 6.1.0-new"),
                (1_700_500_000, "shut down"),
            ]
        );
    }

    #[test]
    fn the_same_boot_in_both_files_and_proc_shows_once() {
        let root = TestRoot::new("boot-both");
        root.bytes("/var/lib/wtmpdb/wtmp.db", DB);
        root.bytes(
            "/var/log/wtmp",
            &record(2, "reboot", "6.12.5-test", (BOOT2 + 3) as u32),
        );
        root.file("/proc/stat", &format!("cpu 1 2 3\nbtime {}\n", BOOT2 - 2));
        root.file("/proc/sys/kernel/osrelease", "6.12.5-test\n");
        let r = collect(&root.0, DOWN1 + 1);
        assert_eq!(r.events.len(), 1, "{:?}", r.events);
        assert_eq!(r.events[0].text, "booted 6.12.5-test");
    }

    #[test]
    fn falls_back_to_proc_for_the_running_boot() {
        let root = TestRoot::new("boot-proc");
        root.file("/proc/stat", "cpu 1 2 3\nbtime 1714550405\n");
        root.file("/proc/sys/kernel/osrelease", "6.12.4-test\n");
        assert_eq!(lines(&collect(&root.0, 0)), [(BOOT1, "booted 6.12.4-test")]);
        // Nothing at all (a mounted disk without records) is quiet.
        let root = TestRoot::new("boot-none");
        let r = collect(&root.0, 0);
        assert!(r.events.is_empty() && r.notes.is_empty());
    }

    #[test]
    fn a_damaged_database_is_a_note() {
        let root = TestRoot::new("boot-bad");
        root.bytes("/var/lib/wtmpdb/wtmp.db", &DB[..700]);
        let r = collect(&root.0, 0);
        assert!(r.events.is_empty());
        assert_eq!(r.notes.len(), 1);
        assert!(r.notes[0].starts_with("boot: can't read"), "{}", r.notes[0]);
    }
}
