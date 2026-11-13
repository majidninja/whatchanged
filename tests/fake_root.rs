//! Runs the real binary against a small fake system under `--root`, with
//! every source given something to find. This checks the whole path from
//! the command line to the printed timeline, without touching the real
//! machine's logs.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fresh directory under cargo's per-test scratch space.
fn fresh(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn file(root: &Path, path: &str, contents: impl AsRef<[u8]>) {
    let full = root.join(path);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(full, contents).unwrap();
}

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_whatchanged"))
        .arg("--root")
        .arg(root)
        .args(args)
        .env("TZ", "UTC")
        .output()
        .unwrap()
}

const DPKG_LOG: &str = "\
2024-05-01 08:00:01 startup archives unpack
2024-05-01 09:12:30 upgrade mesa:amd64 24.1-1 24.2-1
2024-05-01 09:12:31 status half-configured mesa:amd64 24.2-1
2024-05-01 09:13:02 configure mesa:amd64 24.2-1 <none>
2024-05-02 10:00:00 install htop:amd64 <none> 3.3.0-1
";

const APT_HISTORY: &str = "
Start-Date: 2024-05-01  09:12:20
Commandline: apt upgrade
Requested-By: alice (1000)
Upgrade: mesa:amd64 (24.1-1, 24.2-1)
End-Date: 2024-05-01  09:13:05
";

const PACMAN_LOG: &str = "\
[2024-05-03T09:12:00+0000] [PACMAN] Running 'pacman -Syu'
[2024-05-03T09:12:31+0000] [ALPM] upgraded linux (6.12.4.arch1-1 -> 6.12.5.arch1-1)
[2024-05-03T09:12:40+0000] [ALPM] transaction completed
";

const DNF_LOG: &str = "\
2024-05-04T09:12:30+0000 SUBDEBUG Upgrade: mesa-dri-drivers-24.2-1.fc40.x86_64
2024-05-04T09:12:40+0000 SUBDEBUG Upgraded: mesa-dri-drivers-24.1-3.fc40.x86_64
";

/// A fake machine with something for every source. Log entries are from
/// May 2024; files created here get the current time as their mtime.
fn full_tree(root: &Path) {
    file(root, "var/log/dpkg.log", DPKG_LOG);
    file(root, "var/log/apt/history.log", APT_HISTORY);
    // A compressed rotated log, as logrotate leaves them:
    // "2024-04-02 08:00:00 install oldpkg:amd64 <none> 1.0-1\n" as gzip.
    let old_dpkg: &[u8] = &[
        0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x03, 0x33, 0x32, 0x30, 0x32, 0xd1,
        0x35, 0x00, 0x22, 0x23, 0x05, 0x03, 0x0b, 0x2b, 0x03, 0x03, 0x20, 0x52, 0xc8, 0xcc, 0x2b,
        0x2e, 0x49, 0xcc, 0xc9, 0x51, 0xc8, 0xcf, 0x49, 0x29, 0xc8, 0x4e, 0xb7, 0x4a, 0xcc, 0x4d,
        0x31, 0x33, 0x51, 0xb0, 0xc9, 0xcb, 0xcf, 0x4b, 0xb5, 0x53, 0x30, 0xd4, 0x33, 0xd0, 0x35,
        0xe4, 0x02, 0x00, 0x85, 0x55, 0x97, 0xd5, 0x36, 0x00, 0x00, 0x00,
    ];
    fs::write(root.join("var/log/dpkg.log.2.gz"), old_dpkg).unwrap();
    file(root, "var/log/pacman.log", PACMAN_LOG);
    file(root, "var/log/dnf.rpm.log", DNF_LOG);

    let branch = root.join("var/lib/flatpak/app/org.example.Editor/x86_64/stable");
    fs::create_dir_all(&branch).unwrap();
    symlink("abc123", branch.join("active")).unwrap();
    file(root, "var/lib/snapd/snaps/hello_42.snap", "");

    file(root, "usr/lib/modules/6.12.5-test/modules.dep", "");
    file(root, "boot/vmlinuz-6.12.5-test", "");

    file(root, "etc/pipewire/pipewire.conf", "x");
    file(root, "home/alice/.config/foo/foo.conf", "x");

    file(root, "etc/profile.d/proxy.sh", "export X=1");
    file(root, "home/alice/.bashrc", "alias ll='ls -l'");

    file(root, "etc/systemd/system/backup.service", "[Unit]");
    fs::create_dir_all(root.join("etc/systemd/system/multi-user.target.wants")).unwrap();
    symlink(
        "/etc/systemd/system/backup.service",
        root.join("etc/systemd/system/multi-user.target.wants/backup.service"),
    )
    .unwrap();

    file(root, "etc/crontab", "0 3 * * * root /backup");
}

#[test]
fn every_source_shows_up() {
    let root = fresh("full");
    full_tree(&root);
    let out = run(&root, &["--since", "2024-01-01"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "stderr: {stderr}");

    for want in [
        "mesa 24.1-1 -> 24.2-1",
        "(apt upgrade, by alice)",
        "installed oldpkg 1.0-1",
        "htop",
        "(pacman -Syu)",
        "mesa-dri-drivers",
        "org.example.Editor",
        "hello",
        "6.12.5-test",
        "/etc/pipewire/pipewire.conf",
        "foo.conf",
        "/etc/profile.d/proxy.sh",
        ".bashrc",
        "backup.service",
        "/etc/crontab",
    ] {
        assert!(stdout.contains(want), "missing {want:?} in:\n{stdout}");
    }
    // Times in the timeline are in $TZ, which we set to UTC.
    assert!(stdout.contains("2024-05-01\n  09:12  pkg"), "{stdout}");
    // Every kind of event is there.
    for kind in ["pkg", "kernel", "config", "env", "service", "cron"] {
        let tag = format!("  {kind:<7} ");
        assert!(stdout.contains(&tag), "no {kind} line in:\n{stdout}");
    }
}

#[test]
fn only_and_until_narrow_the_timeline() {
    let root = fresh("narrow");
    full_tree(&root);
    let out = run(
        &root,
        &[
            "--since",
            "2024-05-01",
            "--until",
            "2024-05-02",
            "--only",
            "pkg",
        ],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success());
    assert!(stdout.contains("mesa 24.1-1 -> 24.2-1"), "{stdout}");
    for not in ["htop", "pacman", "/etc/crontab", ".bashrc", "hello"] {
        assert!(!stdout.contains(not), "unexpected {not:?} in:\n{stdout}");
    }
}

#[test]
fn empty_root_says_nothing_changed_and_what_it_skipped() {
    let root = fresh("empty");
    let out = run(&root, &["--since", "2024-01-01"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "stderr: {stderr}");
    assert!(
        stdout.starts_with("Nothing changed since 2024-01-01"),
        "{stdout}"
    );
    assert!(stderr.contains("Skipped:"), "{stderr}");
}

#[test]
fn odd_and_large_logs_do_not_crash() {
    let root = fresh("odd");
    let mut log = Vec::new();
    // Garbage, invalid UTF-8, a very long line, half lines, then one good line.
    log.extend_from_slice(b"\xff\xfe not text at all\n");
    log.extend(std::iter::repeat_n(b'x', 1_000_000));
    log.extend_from_slice(b"\n2024-05-01 09:12\n2024-05-01 99:99:99 upgrade a:amd64 1 2\n");
    for i in 0..50_000 {
        log.extend_from_slice(
            format!("2023-01-01 00:00:00 status installed p{i}:amd64 1\n").as_bytes(),
        );
    }
    log.extend_from_slice(b"2024-05-01 09:12:30 upgrade mesa:amd64 24.1-1 24.2-1\n");
    file(&root, "var/log/dpkg.log", &log);
    file(
        &root,
        "var/log/pacman.log",
        b"[broken\n[2024-13-45T99:00:00+0000] [ALPM] upgraded\n",
    );
    file(
        &root,
        "var/log/apt/history.log",
        "Start-Date: nonsense\nCommandline:\n",
    );

    let out = run(&root, &["--since", "2024-01-01", "--only", "pkg"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("mesa 24.1-1 -> 24.2-1"), "{stdout}");
}

#[test]
fn bad_arguments_exit_2() {
    let root = fresh("args");
    for args in [
        &["--since", "someday"][..],
        &["--only", "nope"],
        &["--since", "2024-05-02", "--until", "2024-05-01"],
        &["--frobnicate"],
    ] {
        let out = run(&root, args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("--help"));
    }
    let out = run(&root.join("missing"), &[]);
    assert_eq!(out.status.code(), Some(2));
}
