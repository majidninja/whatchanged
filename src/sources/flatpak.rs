//! Flatpak apps and runtimes, system-wide (`/var/lib/flatpak`) and per user
//! (`~/.local/share/flatpak`).
//!
//! Flatpak keeps no plain-text log, but every deployed ref has an `active`
//! symlink, `<repo>/app/<id>/<arch>/<branch>/active`, pointing at the
//! deployed commit. Flatpak replaces that symlink on every install or
//! update, so its own mtime (not the target's) is when the ref last changed.
//! We can't tell an install from an update this way, and uninstalls leave
//! nothing behind.
//!
//! A machine without flatpak is normal, so a missing directory is silent.

use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::sources::pkg::{self, Action};
use crate::sources::{shell::homes, under};
use crate::timeline::SourceResult;

const SYSTEM_REPO: &str = "/var/lib/flatpak";
const USER_REPO: &str = ".local/share/flatpak";

pub fn collect(root: &Path, since: i64) -> SourceResult {
    let mut result = SourceResult::default();
    let mut actions = Vec::new();
    let mut repos = vec![(SYSTEM_REPO.to_string(), false)];
    repos.extend(
        homes(root)
            .into_iter()
            .map(|h| (format!("{h}/{USER_REPO}"), true)),
    );
    for (repo, user) in repos {
        for kind in ["app", "runtime"] {
            scan(
                root,
                &format!("{repo}/{kind}"),
                kind,
                user,
                &mut actions,
                &mut result.notes,
            );
        }
    }
    actions.retain(|a| a.time >= since);
    actions.sort_by(|a, b| (a.time, &a.detail).cmp(&(b.time, &b.detail)));
    result.events = pkg::group(actions.into_iter(), since);
    result
}

/// Walks `<dir>/<id>/<arch>/<branch>/active` and records one action per ref.
fn scan(
    root: &Path,
    dir: &str,
    kind: &str,
    user: bool,
    actions: &mut Vec<Action>,
    notes: &mut Vec<String>,
) {
    let full = under(root, dir);
    let ids = match std::fs::read_dir(&full) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return,
        Err(e) => {
            notes.push(format!("flatpak: can't read {}: {e}", full.display()));
            return;
        }
    };
    for id in ids.flatten() {
        let id_name = id.file_name().to_string_lossy().into_owned();
        for arch in subdirs(&id.path()) {
            for branch in subdirs(&arch) {
                let Ok(meta) = std::fs::symlink_metadata(branch.join("active")) else {
                    continue;
                };
                let branch_name = branch.file_name().unwrap_or_default().to_string_lossy();
                // Runtimes come in many branches side by side (GNOME 46, 47),
                // so name them the way flatpak does: id//branch.
                let mut detail = if kind == "runtime" {
                    format!("{id_name}//{branch_name}")
                } else {
                    id_name.clone()
                };
                if user {
                    detail.push_str(" (user)");
                }
                actions.push(Action {
                    time: meta.mtime(),
                    verb: "flatpak",
                    detail,
                });
            }
        }
    }
}

fn subdirs(dir: &Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<_> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect();
    v.sort();
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::TestRoot;
    use std::os::unix::fs::symlink;

    fn deploy(root: &TestRoot, refdir: &str) {
        let dir = under(&root.0, refdir);
        root.file(&format!("{refdir}/abc123/metadata"), "");
        symlink("abc123", dir.join("active")).unwrap();
    }

    #[test]
    fn finds_apps_and_runtimes() {
        let root = TestRoot::new("flatpak");
        deploy(
            &root,
            "/var/lib/flatpak/app/org.mozilla.firefox/x86_64/stable",
        );
        deploy(
            &root,
            "/var/lib/flatpak/runtime/org.gnome.Platform/x86_64/47",
        );
        deploy(
            &root,
            "/home/alice/.local/share/flatpak/app/org.gimp.GIMP/x86_64/stable",
        );
        // Half-written deploy without an active link: ignored.
        root.file(
            "/var/lib/flatpak/app/org.broken/x86_64/stable/x/metadata",
            "",
        );

        let r = collect(&root.0, 0);
        assert!(r.notes.is_empty());
        let all: String = r.events.iter().map(|e| e.text.clone() + "\n").collect();
        assert!(all.contains("org.mozilla.firefox"));
        assert!(all.contains("org.gnome.Platform//47"));
        assert!(all.contains("org.gimp.GIMP (user)"));
        assert!(!all.contains("org.broken"));
        assert!(all.starts_with("flatpak   "));

        assert!(collect(&root.0, i64::MAX).events.is_empty());
    }

    #[test]
    fn silent_without_flatpak() {
        let root = TestRoot::new("flatpak-none");
        let r = collect(&root.0, 0);
        assert!(r.events.is_empty());
        assert!(r.notes.is_empty());
    }
}
