# DEVLOG

Notes to self between sessions. Read this first.

## Roadmap

1. [x] Timeline model (`Event`, `SourceResult`), plain-text renderer with day headers
2. [x] `--since` parsing: today, yesterday, 30m/2h/3d/1w, ISO dates
3. [x] dpkg/apt source (`/var/log/dpkg.log`, `.log.1`), grouped per minute
4. [x] Local time zone: parse `/etc/localtime` (TZif) or honour `TZ`; currently UTC
5. [ ] Rotated gzip logs (`dpkg.log.2.gz` ...) — needs a tiny inflate or a crate (`flate2`?); decide
6. [x] Kernels: `/lib/modules/*` + ctime of the vmlinuz image
7. [ ] Config files: mtimes under `/etc` and `~/.config` (skip noisy paths, depth limit)
8. [ ] systemd: unit files and `*.wants/` symlinks in `/etc/systemd` and `~/.config/systemd`
9. [ ] Shell startup files: `~/.bashrc`, `~/.profile`, `~/.zshrc`, `/etc/profile.d`
10. [ ] Crontabs: `/etc/crontab`, `/etc/cron.d`, `/var/spool/cron` (may need root)
11. [ ] Other package managers: pacman.log, dnf (history db is sqlite — maybe `dnf.rpm.log`), flatpak
12. [ ] CLI: `--only pkg,config`, `--until`; man page
13. [x] Sources take a root path (`--root DIR`); `TestRoot` scratch dir for tests
14. [ ] With `--root`, read the time zone from `<root>/etc/localtime` too

## Layout

- `src/time.rs` — std-only calendar math (Hinnant's days_from_civil)
- `src/tz.rs` — `Tz`: TZif reader + POSIX TZ rule strings; `Tz::local()`, `offset_at`, `local_time`, `to_epoch`
- `src/since.rs` — `--since` parsing
- `src/timeline.rs` — `Event`, `SourceResult`, `render`
- `src/sources/mod.rs` — `collect_all`, `read_logs` helper (missing = silent, others = note)
- `src/sources/dpkg.rs` — dpkg log parser
- `src/sources/kernel.rs` — installed kernels, running/"not booted yet" markers

## 2026-10-23

First session. `cargo init`, no dependencies. Built the timeline model, renderer
(date header per day, `HH:MM  kind    text`), `--since` parsing, and the dpkg source.
dpkg actions with the same verb in the same minute are merged into one line,
capped at 5 packages + "and N more". Skipped-source notes go to stderr.
Tested against this machine's real dpkg.log; looks right. 9 unit tests, all on sample text.

Next: local time zone (roadmap 4) — everything is UTC right now, which is
wrong for most users. Plan: honour `TZ=UTC`/fixed offsets, else read the TZif
file at `/etc/localtime` and find the offset for a given timestamp. Note that
`local_offset()` returning one number is too simple across DST changes; make it
`offset_at(ts)`. After that: make sources take a root path for testability.

## 2026-10-24

Local time zones (roadmap 4), still no dependencies. New `src/tz.rs`:
- Reads TZif (v1, and the 64-bit v2+ section), plus the POSIX rule footer
  used for times after the last transition. This matters because "slim"
  zone files hold almost no transitions and rely on the footer.
- POSIX rules support `Mm.w.d`, `Jn` and `n` day forms, `/time`, quoted
  `<+0530>` names, and southern-hemisphere DST (start after end).
- `Tz::local()`: `$TZ` (`:` prefix, absolute path, or name under
  /usr/share/zoneinfo, or a raw POSIX string), else /etc/localtime. If that
  fails it falls back to UTC and adds a "Skipped:" note.
- All `offset: i64` params are now `&Tz`. `today`/`yesterday` go through the
  calendar, so DST days (23h/25h) give the correct midnight.
- Tests: synthetic TZif built in the test, POSIX Europe/Sydney rules, plus one
  `#[ignore]` test against real /usr/share/zoneinfo (`cargo test -- --include-ignored`).
  Ran it here; it passes. 14 regular tests + 1 ignored.

Note: dpkg logs use local time and we print local time, so dpkg output looks
the same in every zone. Zones will matter once mtime-based sources (UTC
epochs) land.

Next: make sources take a root path (roadmap 13), then the kernels source
(roadmap 6): the first mtime-based source, which finally exercises the time zone code.
Known limit: a local time that happens twice (the DST fall-back hour) is read
as the later of the two. Fine for now.

## 2026-10-25

Two things, still no dependencies:

- **Root path** (roadmap 13). Every source takes `root: &Path`; system paths
  are joined with `sources::under(root, "/var/log/...")`. New CLI flag
  `--root DIR` for inspecting a mounted broken system from a rescue USB.
  Tests use `sources::TestRoot` (temp dir, removed on drop) to build a fake
  filesystem. dpkg got a `collects_from_root` test.
- **Kernel source** (roadmap 6). Lists `/lib/modules/*` (or `/usr/lib/modules`),
  keeps only versions that still have an image (`/boot/vmlinuz-<v>` or
  `/lib/modules/<v>/vmlinuz`) and uses the image's **ctime**: dpkg/rpm keep
  the build-time mtime, ctime is when it hit this disk. Found on the real
  machine: removed kernels leave their modules dir behind, hence the image check.
  Marks `(running)` from `/proc/sys/kernel/osrelease`, and `(not booted yet)`
  if installed after `btime` in `/proc/stat`. Under `--root /mnt` there's no
  /proc, so no markers — that's intended.
  21 tests + 1 ignored.

Known limits: ctime also changes on chmod/chown or restore from backup.
With `--root`, the time zone still comes from the host (roadmap 14).

Next: roadmap 14 is small (`Tz::local` taking a root; `$TZ` only when root is
`/`). Then config files under /etc (roadmap 7): walk with a depth limit, skip
noisy paths (/etc/ld.so.cache, /etc/mtab, ...), group many files changed in the
same minute (like dpkg does), and drop files that dpkg itself touched in that
minute so an upgrade doesn't show 40 "config edited" lines.
