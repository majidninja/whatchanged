//! Shell startup files: `~/.bashrc`, `~/.profile`, `/etc/profile.d/*`, ...
//! A stray line in one of these is a classic cause of "my terminal broke"
//! or "PATH is wrong", so any edit is worth showing.
//!
//! We use the mtime, which is when the contents were last written. Home
//! directories are `/root` and every directory in `/home`, so this also
//! works on a mounted disk where `$HOME` means nothing.

use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::sources::under;
use crate::timeline::{Event, SourceResult};

pub const SYSTEM_FILES: &[&str] = &[
    "/etc/profile",
    "/etc/bash.bashrc",
    "/etc/bashrc",
    "/etc/environment",
    "/etc/zsh/zshrc",
    "/etc/zsh/zshenv",
    "/etc/zshrc",
];

pub const SYSTEM_DIRS: &[&str] = &["/etc/profile.d"];

const HOME_FILES: &[&str] = &[
    ".profile",
    ".bashrc",
    ".bash_profile",
    ".bash_login",
    ".bash_logout",
    ".zshrc",
    ".zshenv",
    ".zprofile",
    ".config/fish/config.fish",
    ".pam_environment",
];

pub fn collect(root: &Path, since: i64) -> SourceResult {
    let mut result = SourceResult::default();
    let mut files: Vec<String> = SYSTEM_FILES.iter().map(|s| s.to_string()).collect();
    for dir in SYSTEM_DIRS {
        files.extend(list_dir(root, dir, &mut result.notes));
    }
    for home in homes(root) {
        for f in HOME_FILES {
            files.push(format!("{home}/{f}"));
        }
    }

    let mut denied = Vec::new();
    for path in files {
        match std::fs::metadata(under(root, &path)) {
            Ok(m) if m.is_file() && m.mtime() >= since => result.events.push(Event {
                time: m.mtime(),
                kind: "env",
                text: format!("{path} edited"),
            }),
            Ok(_) => {}
            // Typically /root as a normal user: say so once per directory.
            Err(e) if e.kind() == ErrorKind::PermissionDenied => {
                let dir = under(root, &path).parent().map(Path::to_path_buf);
                if let Some(dir) = dir.filter(|d| !denied.contains(d)) {
                    result
                        .notes
                        .push(format!("env: no permission to look in {}", dir.display()));
                    denied.push(dir);
                }
            }
            Err(_) => {}
        }
    }
    result
}

/// Home directories as paths relative to `root`: `/root` and `/home/*`.
pub fn homes(root: &Path) -> Vec<String> {
    let mut homes = vec!["/root".to_string()];
    if let Ok(entries) = std::fs::read_dir(under(root, "/home")) {
        let mut users: Vec<String> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|e| e.file_name().into_string().ok())
            .map(|name| format!("/home/{name}"))
            .collect();
        users.sort();
        homes.extend(users);
    }
    homes
}

/// Entries of a directory, as paths relative to `root`. Missing is fine.
fn list_dir(root: &Path, dir: &str, notes: &mut Vec<String>) -> Vec<String> {
    let full = under(root, dir);
    match std::fs::read_dir(&full) {
        Ok(entries) => {
            let mut v: Vec<String> = entries
                .flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .map(|name| format!("{dir}/{name}"))
                .collect();
            v.sort();
            v
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Vec::new(),
        Err(e) => {
            notes.push(format!("env: can't read {}: {e}", full.display()));
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::TestRoot;

    #[test]
    fn finds_system_and_home_files() {
        let root = TestRoot::new("shell");
        root.file("/etc/profile", "");
        root.file("/etc/profile.d/proxy.sh", "");
        root.file("/home/alice/.bashrc", "");
        root.file("/home/bob/.config/fish/config.fish", "");
        root.file("/home/bob/.vimrc", ""); // not a shell file
        root.file("/root/.profile", "");

        let r = collect(&root.0, 0);
        let mut texts: Vec<&str> = r.events.iter().map(|e| e.text.as_str()).collect();
        texts.sort();
        assert_eq!(
            texts,
            [
                "/etc/profile edited",
                "/etc/profile.d/proxy.sh edited",
                "/home/alice/.bashrc edited",
                "/home/bob/.config/fish/config.fish edited",
                "/root/.profile edited",
            ]
        );
        assert!(r.notes.is_empty());
        assert!(r.events.iter().all(|e| e.kind == "env"));

        // All files were written just now; a future `since` hides them.
        assert!(collect(&root.0, i64::MAX).events.is_empty());
    }

    #[test]
    fn empty_root_is_quiet() {
        let root = TestRoot::new("shell-empty");
        let r = collect(&root.0, 0);
        assert!(r.events.is_empty());
        assert!(r.notes.is_empty());
    }
}
