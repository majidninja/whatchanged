# DEVLOG

Notes to self between sessions. Read this first.

## Roadmap

1. [x] Timeline model (`Event`, `SourceResult`), plain-text renderer with day headers
2. [x] `--since` parsing: today, yesterday, 30m/2h/3d/1w, ISO dates
3. [x] dpkg/apt source (`/var/log/dpkg.log`, `.log.1`), grouped per minute
4. [x] Local time zone: parse `/etc/localtime` (TZif) or honour `TZ`; currently UTC
5. [ ] Rotated gzip logs (`dpkg.log.2.gz` ...) — needs a tiny inflate or a crate (`flate2`?); decide
6. [x] Kernels: `/lib/modules/*` + ctime of the vmlinuz image
7. [x] Config files: mtimes under `/etc` and each `~/.config` (skip noisy paths, depth limit)
8. [ ] systemd: unit files and `*.wants/` symlinks in `/etc/systemd` and `~/.config/systemd`
9. [x] Shell startup files: `~/.bashrc`, `~/.profile`, `~/.zshrc`, `/etc/profile.d`
10. [ ] Crontabs: `/etc/crontab`, `/etc/cron.d`, `/var/spool/cron` (may need root)
11. [ ] Other package managers: pacman.log, dnf (history db is sqlite — maybe `dnf.rpm.log`), flatpak
12. [ ] CLI: `--only pkg,config`, `--until`; man page
13. [x] Sources take a root path (`--root DIR`); `TestRoot` scratch dir for tests
14. [x] With `--root`, read the time zone from `<root>/etc/localtime` too

## Layout

- `src/time.rs` — std-only calendar math (Hinnant's days_from_civil)
- `src/tz.rs` — `Tz`: TZif reader + POSIX TZ rule strings; `Tz::local()`, `offset_at`, `local_time`, `to_epoch`
- `src/since.rs` — `--since` parsing
- `src/timeline.rs` — `Event`, `SourceResult`, `render`
- `src/sources/mod.rs` — `collect_all`, `read_logs` helper (missing = silent, others = note)
- `src/sources/dpkg.rs` — dpkg log parser
- `src/sources/kernel.rs` — installed kernels, running/"not booted yet" markers
- `src/sources/etc.rs` — files under /etc (depth 6, `NOISY`) and each `~/.config` (depth 2, `NOISY_APPS`, `NOISY_NAMES`); a `Walker` with a skip closure, no symlinks; grouped per minute, kind `config`
- `src/sources/shell.rs` — shell startup files (system + `/root` + `/home/*`), mtime, kind `env`

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

## 2026-10-26

Two small steps, still no dependencies:

- **Time zone under `--root`** (roadmap 14). `Tz::local(root)`: `$TZ` is only
  honoured when root is `/`; otherwise `<root>/etc/localtime` is read. That
  file is normally an *absolute* symlink (`/usr/share/zoneinfo/...`), which
  would silently resolve into the host's zoneinfo, so `resolve_under` follows
  links by hand and re-roots absolute targets (max 8 hops).
- **Shell startup files** (roadmap 9). New `sources/shell.rs`, kind `env`.
  System files (`/etc/profile`, `/etc/bash.bashrc`, `/etc/environment`, zsh
  files, everything in `/etc/profile.d`) plus per-home dotfiles for `/root`
  and every dir in `/home` (no `$HOME`, so it works with `--root`). Uses
  mtime. Permission denied (e.g. `/root` as a user) gives one note per
  directory; missing files are silent. Paths are printed in full
  (`/home/alice/.bashrc`) since there can be several users.
  25 tests + 1 ignored.

Seen on the real machine: a new user's `.profile` and `.bashrc` show as two
`env` lines in the same minute (copied from /etc/skel). Could merge same-minute
`env` lines like dpkg does; low priority.

Next: config files under /etc and ~/.config (roadmap 7): walk with a depth
limit, skip noisy paths (/etc/ld.so.cache, /etc/mtab, ...), group many files
changed in the same minute, drop files dpkg touched in that minute. The shell
source's files overlap /etc — the config walker should skip those paths.

## 2026-10-27

**Config files under /etc** (first half of roadmap 7), still no dependencies.
New `sources/etc.rs`, kind `config`:
- Walks `/etc` up to 6 levels deep, sorted, using `symlink_metadata` so
  symlinks (alternatives, certs, `/etc/resolv.conf` links) are never followed.
- Skips a `NOISY` list (mtab, ld.so.cache, adjtime, resolv.conf, `passwd-`
  style backups, .pwd.lock) and the shell source's paths
  (`shell::SYSTEM_FILES` / `SYSTEM_DIRS` are now `pub`), so nothing shows twice.
- Files with an mtime in the same minute become one line, 5 names max + "and N more".
- Unreadable directories (e.g. /etc/ssl/private as a user) become a single
  note: "no permission to look in X and N more".
- Tests set mtimes with `File::set_modified`, so they don't depend on the
  clock. 29 tests + 1 ignored.

On the real machine over 3 days it printed two lines (gitlab-secrets.json,
openvpn's ipp.txt). ipp.txt is runtime state, not config; if more files like
that show up, extend `NOISY`.

Not done: "drop files dpkg touched in that minute". Grouping already keeps an
upgrade to one line; revisit if it's still too noisy in practice.

Next: `~/.config` for each home (second half of roadmap 7). Reuse
`shell::homes` (make it `pub`). Needs a stronger skip list: browser profiles,
caches and editor state rewrite files constantly. Maybe depth 3 and skip known
app dirs (chromium, google-chrome, mozilla, Code, discord, ...).
Then systemd units (roadmap 8).

## 2026-10-28

**`~/.config` for each home** (second half of roadmap 7, now done), no new
dependencies. In `sources/etc.rs`:
- The walk became a small `Walker` struct (root, since, files, denied, notes)
  with `walk(dir, depth, skip)`, where `skip(path, name)` is a closure. That
  keeps clippy's argument-count lint quiet and lets /etc and homes use
  different rules. `depth` now counts down (levels of subdirs still allowed).
- Homes come from `shell::homes` (now `pub`): `/root` and `/home/*`.
  Depth 2 below `.config` (`app/dir/file`).
- Skipped: `NOISY_APPS` directly under `.config` (browsers, Electron chat
  apps, editors, `pulse`, `dconf`, `session`), `NOISY_NAMES` anywhere (cache
  dirs, lock files, `fish_variables`, `htoprc`), names ending in `~`, `.swp`,
  `.tmp`, and `fish/config.fish` (shell source has it).
- /etc and home files share one per-minute grouping, so they can show on the
  same line. Fine for now; maybe split per home later if it reads badly.
  30 tests + 1 ignored.

Couldn't look at this machine's real `~/.config` (sandboxed), so the skip
lists are from experience, not observation. Watch for noise and extend.
`dconf/user` is skipped although GNOME settings changes live there; it's a
binary db rewritten on any tweak. Could show it as one "GNOME settings
changed" line later.

Next: systemd units (roadmap 8): unit files in `/etc/systemd/system` and
`~/.config/systemd/user` (these are under the config walk today, so skip
them there to avoid showing twice), and `*.wants/` symlinks (use the
symlink's own mtime via `symlink_metadata`) → "docker.service enabled".
