//! Configuration: files under `/etc` and each user's `~/.config` whose
//! contents changed.
//!
//! We walk those trees (not following symlinks, with a depth limit) and use
//! each regular file's mtime. Files rewritten by themselves all the time
//! (`/etc/mtab`, browser profiles, caches, ...) are skipped, and so are the
//! shell startup files, which the `shell` source already reports as `env`,
//! and systemd unit directories, which the `service` source reports.
//!
//! Files edited in the same minute become one line, because one command
//! (an upgrade, `update-ca-certificates`, `useradd`) often rewrites several.

use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::sources::{shell, systemd, under};
use crate::timeline::{Event, SourceResult};

/// How many directory levels below /etc we look. Deeper trees are rare and
/// tend to be generated data, not hand-written config.
const MAX_DEPTH: usize = 6;

/// How many file names one line lists before "and N more".
const MAX_SHOWN: usize = 5;

/// Files that change on their own during normal operation.
const NOISY: &[&str] = &[
    "/etc/mtab",
    "/etc/ld.so.cache",
    "/etc/adjtime",
    "/etc/resolv.conf",
    "/etc/.pwd.lock",
    "/etc/passwd-",
    "/etc/group-",
    "/etc/shadow-",
    "/etc/gshadow-",
    "/etc/subuid-",
    "/etc/subgid-",
];

/// Directories directly under `~/.config` that belong to browsers, chat apps
/// and editors. They rewrite their files constantly and would drown the rest.
const NOISY_APPS: &[&str] = &[
    "BraveSoftware",
    "chromium",
    "google-chrome",
    "microsoft-edge",
    "vivaldi",
    "Code",
    "Code - OSS",
    "VSCodium",
    "Cursor",
    "discord",
    "Slack",
    "Signal",
    "Element",
    "spotify",
    "teams-for-linux",
    "obsidian",
    "JetBrains",
    "pulse",
    "dconf",
    "session",
    "procps",
];

/// Names under `~/.config` that are runtime state wherever they appear.
const NOISY_NAMES: &[&str] = &[
    "Cache",
    "cache",
    "GPUCache",
    "Code Cache",
    "Crashpad",
    "logs",
    "lock",
    ".lock",
    "fish_variables",
    "htoprc",
];

/// How many directory levels below each `~/.config` we look:
/// `app/dir/file` is plenty.
const HOME_DEPTH: usize = 2;

pub fn collect(root: &Path, since: i64) -> SourceResult {
    let mut w = Walker {
        root,
        since,
        files: Vec::new(),
        denied: Vec::new(),
        result: SourceResult::default(),
    };
    w.walk("/etc", MAX_DEPTH, &|path, _| is_skipped_etc(path));
    for home in shell::homes(root) {
        let base = format!("{home}/.config/");
        let skip = |path: &str, name: &str| {
            let rest = path.strip_prefix(&base).unwrap_or(path);
            is_noisy_home_name(name)
                || NOISY_APPS.contains(&rest)
                || rest == "fish/config.fish"
                || rest == "systemd/user"
        };
        w.walk(&format!("{home}/.config"), HOME_DEPTH, &skip);
    }

    let Walker {
        files,
        denied,
        mut result,
        ..
    } = w;
    if !denied.is_empty() {
        let more = match denied.len() {
            1 => String::new(),
            n => format!(" and {} more", n - 1),
        };
        result.notes.push(format!(
            "config: no permission to look in {}{more}",
            under(root, &denied[0]).display()
        ));
    }
    result.events = group(files);
    result
}

/// Collects `(mtime, path)` of regular files modified at or after `since`.
struct Walker<'a> {
    root: &'a Path,
    since: i64,
    files: Vec<(i64, String)>,
    denied: Vec<String>,
    result: SourceResult,
}

impl Walker<'_> {
    /// Walks `dir` and at most `depth` levels of subdirectories.
    /// `skip(path, name)` leaves out a file or a whole directory.
    fn walk(&mut self, dir: &str, depth: usize, skip: &dyn Fn(&str, &str) -> bool) {
        let full = under(self.root, dir);
        let entries = match std::fs::read_dir(&full) {
            Ok(e) => e,
            Err(e) if e.kind() == ErrorKind::NotFound => return,
            Err(e) if e.kind() == ErrorKind::PermissionDenied => {
                self.denied.push(dir.to_string());
                return;
            }
            Err(e) => {
                self.result
                    .notes
                    .push(format!("config: can't read {}: {e}", full.display()));
                return;
            }
        };
        let mut names: Vec<String> = entries
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        names.sort();
        for name in names {
            let path = format!("{dir}/{name}");
            if skip(&path, &name) {
                continue;
            }
            // symlink_metadata: don't follow links (alternatives, certs, ...).
            let Ok(meta) = std::fs::symlink_metadata(under(self.root, &path)) else {
                continue;
            };
            if meta.is_dir() {
                if depth > 0 {
                    self.walk(&path, depth - 1, skip);
                }
            } else if meta.is_file() && meta.mtime() >= self.since {
                self.files.push((meta.mtime(), path));
            }
        }
    }
}

fn is_skipped_etc(path: &str) -> bool {
    NOISY.contains(&path)
        || shell::SYSTEM_FILES.contains(&path)
        || shell::SYSTEM_DIRS.contains(&path)
        || systemd::SYSTEM_DIRS.contains(&path)
}

/// Caches, locks and editor leftovers (`foo~`, `.foo.swp`).
fn is_noisy_home_name(name: &str) -> bool {
    NOISY_NAMES.contains(&name)
        || name.ends_with('~')
        || name.ends_with(".swp")
        || name.ends_with(".tmp")
}

/// Turns `(mtime, path)` pairs into events, one per minute.
fn group(mut files: Vec<(i64, String)>) -> Vec<Event> {
    files.sort();
    let mut groups: Vec<(i64, Vec<String>)> = Vec::new();
    for (time, path) in files {
        match groups.last_mut() {
            Some((t, paths)) if *t / 60 == time / 60 => paths.push(path),
            _ => groups.push((time, vec![path])),
        }
    }
    groups
        .into_iter()
        .map(|(time, mut paths)| {
            paths.sort();
            let mut text = paths[..paths.len().min(MAX_SHOWN)].join(", ");
            if paths.len() > MAX_SHOWN {
                text.push_str(&format!(" and {} more", paths.len() - MAX_SHOWN));
            }
            text.push_str(" edited");
            Event {
                time,
                kind: "config",
                text,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::TestRoot;

    #[test]
    fn finds_files_and_skips_noise() {
        let root = TestRoot::new("etc");
        root.file("/etc/fstab", "");
        root.file("/etc/ssh/sshd_config", "");
        root.file("/etc/mtab", ""); // noisy
        root.file("/etc/profile", ""); // belongs to the shell source
        root.file("/etc/profile.d/proxy.sh", ""); // same
        root.file("/etc/systemd/system/x.service", ""); // service source
        root.file("/home/alice/.bashrc", ""); // not under /etc

        // 2024-05-01 09:12:00 and :30 UTC: the same minute.
        set_mtime(&root, "/etc/fstab", 1714554720);
        set_mtime(&root, "/etc/ssh/sshd_config", 1714554750);

        let r = collect(&root.0, 0);
        assert!(r.notes.is_empty());
        let fstab_only = collect(&root.0, 1714554721);
        assert_eq!(fstab_only.events.len(), 1);
        assert_eq!(fstab_only.events[0].text, "/etc/ssh/sshd_config edited");

        let ours: Vec<&Event> = r.events.iter().filter(|e| e.time == 1714554720).collect();
        assert_eq!(ours.len(), 1);
        assert_eq!(ours[0].text, "/etc/fstab, /etc/ssh/sshd_config edited");
        assert_eq!(ours[0].kind, "config");
        // The noisy and shell files were written just now; they must not appear.
        assert!(r.events.iter().all(|e| !e.text.contains("mtab")
            && !e.text.contains("profile")
            && !e.text.contains("systemd")));
    }

    fn set_mtime(root: &TestRoot, path: &str, secs: u64) {
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs);
        std::fs::File::options()
            .write(true)
            .open(under(&root.0, path))
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    #[test]
    fn does_not_follow_symlinks() {
        let root = TestRoot::new("etc-link");
        root.file("/usr/share/thing/a.conf", "");
        std::fs::create_dir_all(under(&root.0, "/etc")).unwrap();
        std::os::unix::fs::symlink(
            under(&root.0, "/usr/share/thing"),
            under(&root.0, "/etc/thing"),
        )
        .unwrap();
        assert!(collect(&root.0, 0).events.is_empty());
    }

    #[test]
    fn groups_by_minute() {
        let files = vec![
            (120, "/etc/b".to_string()),
            (61, "/etc/x".to_string()),
            (125, "/etc/a".to_string()),
            (130, "/etc/c".to_string()),
            (131, "/etc/d".to_string()),
            (132, "/etc/e".to_string()),
            (133, "/etc/f".to_string()),
            (134, "/etc/g".to_string()),
        ];
        let events = group(files);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].time, 61);
        assert_eq!(events[0].text, "/etc/x edited");
        assert_eq!(events[1].time, 120);
        assert_eq!(
            events[1].text,
            "/etc/a, /etc/b, /etc/c, /etc/d, /etc/e and 2 more edited"
        );
    }

    #[test]
    fn finds_home_config_and_skips_app_noise() {
        let root = TestRoot::new("etc-home");
        root.file("/home/alice/.config/pipewire/pipewire.conf", "");
        root.file("/home/alice/.config/mimeapps.list", "");
        root.file("/root/.config/git/config", "");
        // Skipped: browser profile, cache dir, editor backup, fish (shell source).
        root.file("/home/alice/.config/chromium/Default/Preferences", "");
        root.file("/home/alice/.config/foo/cache/x", "");
        root.file("/home/alice/.config/foo/foo.conf~", "");
        root.file("/home/alice/.config/fish/config.fish", "");
        root.file("/home/alice/.config/systemd/user/x.service", "");
        // Too deep: below .config/a/b/c/.
        root.file("/home/alice/.config/a/b/c/deep.conf", "");
        for f in [
            "/home/alice/.config/pipewire/pipewire.conf",
            "/home/alice/.config/mimeapps.list",
            "/root/.config/git/config",
        ] {
            set_mtime(&root, f, 1714554720);
        }

        let r = collect(&root.0, 0);
        assert!(r.notes.is_empty());
        assert_eq!(r.events.len(), 1);
        assert_eq!(
            r.events[0].text,
            "/home/alice/.config/mimeapps.list, /home/alice/.config/pipewire/pipewire.conf, \
             /root/.config/git/config edited"
        );
    }

    #[test]
    fn empty_root_is_quiet() {
        let root = TestRoot::new("etc-empty");
        let r = collect(&root.0, 0);
        assert!(r.events.is_empty());
        assert!(r.notes.is_empty());
    }
}
