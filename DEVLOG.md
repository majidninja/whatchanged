# DEVLOG

Notes to self between sessions. Read this first.

## Roadmap

1. [x] Timeline model (`Event`, `SourceResult`), plain-text renderer with day headers
2. [x] `--since` parsing: today, yesterday, 30m/2h/3d/1w, ISO dates
3. [x] dpkg/apt source (`/var/log/dpkg.log`, `.log.1`), grouped per minute
4. [x] Local time zone: parse `/etc/localtime` (TZif) or honour `TZ`; currently UTC
5. [x] Rotated gzip logs (`dpkg.log.2.gz` ...) — own small inflate in `src/gzip.rs`
6. [x] Kernels: `/lib/modules/*` + ctime of the vmlinuz image
7. [x] Config files: mtimes under `/etc` and each `~/.config` (skip noisy paths, depth limit)
8. [x] systemd: unit files and `*.wants/` symlinks in `/etc/systemd` and `~/.config/systemd`
9. [x] Shell startup files: `~/.bashrc`, `~/.profile`, `~/.zshrc`, `/etc/profile.d`
10. [x] Crontabs: `/etc/crontab`, `/etc/cron.d`, `/var/spool/cron` (may need root)
11. [x] Other package managers: pacman.log (done), dnf via `dnf.rpm.log` (done), dnf5 (last change only, done), flatpak (done), snap (done)
12. [x] CLI: `--only pkg,config`, `--until`; man page (`doc/whatchanged.1`)
15. [x] Boots and shutdowns (`boot` source) from wtmpdb / wtmp
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

## 2026-10-29

**systemd units** (roadmap 8, done), no new dependencies. New
`sources/systemd.rs`, kind `service`:
- Looks in `/etc/systemd/system`, `/etc/systemd/user` and each home's
  `~/.config/systemd/user` (homes from `shell::homes`), one level of subdirs.
- Symlinks inside `*.wants/` / `*.requires/` → "X enabled", dated by the
  link's own mtime (`symlink_metadata`). Top-level link to `/dev/null` →
  "X masked". Other top-level links (aliases like display-manager.service)
  are ignored. Regular files (unit files, `foo.service.d/override.conf`) →
  "path edited".
- User units get a " (user)" / " (user alice)" suffix.
- One line per minute and verb; a unit wanted by two targets is listed once.
- The config walker now skips these dirs (`systemd::SYSTEM_DIRS`,
  `systemd/user` under `.config`), so nothing shows twice.
  33 tests + 1 ignored.

Couldn't run the binary on the real machine this time (sandbox), only tests.
Disables aren't shown (the link is gone). A `*.wants/` dir whose mtime is
newer than all its links means something was removed from it; could say
"a unit was disabled in multi-user.target.wants" later.

Next: crontabs (roadmap 10): `/etc/crontab`, `/etc/cron.d/*`,
`/var/spool/cron/crontabs/*` (Debian) and `/var/spool/cron/*` (Fedora/Arch),
kind `cron`; the spool needs root, so add a "no permission" note. Skip
`/etc/crontab` and `/etc/cron.d` in the config walker.

## 2026-10-30

**crontabs** (roadmap 10, done), no new dependencies. New `sources/cron.rs`,
kind `cron`:
- System: `/etc/crontab`, `/etc/anacrontab`, and files in `/etc/cron.d` and
  `/etc/cron.{hourly,daily,weekly,monthly}` (Debian's `.placeholder` skipped).
  Shown by path.
- Users: files in `/var/spool/cron/crontabs` (Debian/Ubuntu) and
  `/var/spool/cron` (Fedora/Arch) → "crontab of alice". Only regular files
  count, so the Debian `crontabs` folder inside `/var/spool/cron` isn't taken
  for a user. No permission → note "user crontabs need root".
- One line per minute: "/etc/crontab, crontab of alice edited".
- The config walker skips `cron::SYSTEM_FILES` / `cron::SYSTEM_DIRS`.
  36 tests + 1 ignored.

Ran it on this machine with `--since 60d`: shows "20:05 cron crontab of
root edited" on 2026-10-22, and no cron noise in the config lines.

Removals (`crontab -r`, deleted cron.d file) aren't shown; same idea as
disabled units: a folder mtime newer than all its files hints at a removal.
Could add a generic "something was removed from DIR" line for cron.d,
*.wants and profile.d later.

Next: pacman log (roadmap 11: `/var/log/pacman.log`, lines like
`[2024-05-01T09:12:00+0200] [ALPM] upgraded mesa (24.1-1 -> 24.2-1)`).
The timestamps carry their own offset, so it should be simpler than dpkg's
local-time parsing. Reuse the dpkg grouping if it fits.

## 2026-10-31

**pacman log** (part of roadmap 11), no new dependencies. New
`sources/pacman.rs`, kind `pkg`:
- Reads `/var/log/pacman.log`; keeps `[ALPM] installed/upgraded/downgraded/
  reinstalled/removed NAME (VERSION)` lines, skips `[PACMAN]` and
  `[ALPM-SCRIPTLET]` lines.
- Timestamps with an offset (`2024-05-01T09:12:00+0200`) use that offset;
  the old offset-less format (`[2019-01-01 12:00]`, and the very old one
  without the `[ALPM]` tag) is read as local time.
- The per-minute grouping moved out of `dpkg::parse` into `dpkg::group`
  (with a public `dpkg::Action`), and pacman reuses it.
- A missing pacman log gives no note (most machines don't use pacman).
  dpkg still notes a missing log, so Arch users will see "dpkg: no log
  found". Better: one note only if *no* package manager log was found.
  40 tests + 1 ignored.

Couldn't run the binary against a fake root (sandbox blocked writing one);
the `collect` test covers the same path.

Next: fix the package-log notes as above (maybe move `group`/`Action` to a
small `sources/pkg.rs` at the same time), then dnf (`/var/log/dnf.rpm.log`:
`2024-05-01T09:12:00+0200 SUBDEBUG Upgrade: mesa-24.2-1.fc40.x86_64`).

## 2026-10-31 (release binaries, done by hand outside the daily session)

Added `.github/workflows/release.yml`: pushing a `v*` tag runs the tests and
builds static musl binaries for x86_64 (ubuntu-latest) and aarch64
(ubuntu-24.04-arm), smoke-tests them and attaches
`whatchanged-vX.Y.Z-<target>.tar.gz` plus `.sha256` files to the GitHub
release. The tag must match `version` in Cargo.toml. First release: v0.1.0.
README got a "prebuilt binaries" install section.

The GitHub Action majidninja/whatchanged-action downloads these release
assets, so keep the asset names as they are. To release: bump `version` in
Cargo.toml, commit, then tag `vX.Y.Z` (tagging is done by hand, not in the
daily session).

## 2026-11-01

**Package-log notes**, no new dependencies. New `sources/pkg.rs`:
- `Action` and `group` (the per-minute grouping) moved there from dpkg.
- `pkg::collect` runs dpkg and pacman together and replaces both in
  `collect_all`. It adds one note, "pkg: no package manager log found
  (looked for .../dpkg.log, .../pacman.log)", only if none of the logs
  exist and no other read error was reported. Arch users no longer see
  "dpkg: no log found".
- `read_logs` no longer emits a "no log found" note itself; it only
  reports permission/read errors.
  43 tests + 1 ignored.

Checked with `--root src` (empty root): exactly one pkg note.

Next: dnf (`/var/log/dnf.rpm.log`: `2024-05-01T09:12:00+0200 SUBDEBUG
Upgrade: mesa-24.2-1.fc40.x86_64`). Add its LOGS to `pkg::collect`.
Note dnf lines carry name-version-release.arch in one token; split off the
arch, then name from version at the last two `-`. Pacman's `parse_time`
offset handling could move to `time.rs` and be reused.

## 2026-11-02

**dnf log** (roadmap 11), no new dependencies. New `sources/dnf.rs`, kind
`pkg`, wired into `pkg::collect` (so it shares the single "no package
manager log found" note):
- Reads `/var/log/dnf.rpm.log.1` then `/var/log/dnf.rpm.log`; keeps
  `SUBDEBUG <What>: name-version-release.arch` lines.
- `split_nevra` drops the arch and splits name from version-release at the
  last two `-`, so `xorg-x11-server-Xorg-21.1.13-1.fc40.x86_64` works.
- `Upgrade:` (new) is paired with the following `Upgraded:` (old, same
  name, within an hour) to give "name old -> new"; same for
  Downgrade/Downgraded. Without the old line it shows just the new version.
- Installed/Install, Reinstalled/Reinstall, Erase/Erased/Removed/Obsoleted
  are mapped; back-to-back duplicates (Install then Installed) are dropped.
- Pacman's offset-aware timestamp parser moved to `Tz::parse_stamp` (in
  `tz.rs`, since it falls back to local time) and is shared by both.
  47 tests + 1 ignored.

Not checked against a real Fedora log (none on this machine); the sample
lines follow the dnf4 format as I remember it. Worth checking on a Fedora
box.

Next: dnf5 (Fedora 41+). Find out what it writes on disk; `/var/log/dnf5.log`
may have transaction lines. If the history is only in
`/usr/lib/sysimage/libdnf5/transaction_history.sqlite`, add a note like
"dnf5 history is in a database we can't read" when that file exists and no
dnf.rpm.log does. Then flatpak.

## 2026-11-03

**dnf5** (Fedora 41+), no new dependencies. dnf5 doesn't write
`dnf.rpm.log`; its history lives only in
`/usr/lib/sysimage/libdnf5/transaction_history.sqlite`, which we can't read
without a sqlite crate. In `sources/dnf.rs`:
- `dnf5_event`: the newest mtime of the database and its `-wal` file
  becomes one `pkg` event, "dnf5 changed packages (details: dnf5 history
  info last)", if it's at or after `since`.
- Skipped when a dnf.rpm.log was written as late or later (dnf4 did the
  work and the log already has the details).
- `pkg::collect` counts the database as a package log, so a dnf5-only
  machine doesn't get "no package manager log found".
  49 tests + 1 ignored.

Limits: only the *last* dnf5 transaction shows up. Earlier ones since
`--since` are invisible. Not checked on a real Fedora 41 box. Also worth
checking: does dnf5 write anything useful to `/var/log/dnf5.log`?

Next: flatpak (roadmap). System installs live in `/var/lib/flatpak`, user
installs in `~/.local/share/flatpak`. Probably mtimes of
`app/<id>/<arch>/<branch>/active` symlinks (that's the deployed commit),
one event per app: "flatpak updated org.mozilla.firefox".

## 2026-11-04

**flatpak** (roadmap), no new dependencies. New `sources/flatpak.rs`, own
entry in `collect_all`, events of kind `pkg` grouped with `pkg::group`
(verb `flatpak`):
- Scans `/var/lib/flatpak/{app,runtime}` and each home's
  `~/.local/share/flatpak/{app,runtime}` for
  `<id>/<arch>/<branch>/active` symlinks; the symlink's own mtime
  (`symlink_metadata`) is when that ref was last deployed.
- Apps are shown by id, runtimes as `id//branch` (flatpak's notation, since
  several branches sit side by side); user installs get ` (user)`.
- No flatpak directory is silent (not an error); unreadable dirs give a note.
  51 tests + 1 ignored.

Limits: can't tell install from update; uninstalls invisible. Not checked
on a real flatpak machine (none here) — worth confirming the `active` link
really is replaced on each update rather than kept.

Next: snap. `/var/lib/snapd/snaps/<name>_<rev>.snap` files: ctime of the
newest revision per snap = install/refresh time; with two revisions present
we could even say "refreshed rev 123 -> 124". Check `/var/lib/snapd/state.json`
too (JSON, root-only?).

## 2026-11-05

**snap** (roadmap 11, now done), no new dependencies. New `sources/snap.rs`,
own entry in `collect_all`, events of kind `pkg` grouped with `pkg::group`
(verb `snap`):
- Lists `/var/lib/snapd/snaps/<name>_<rev>.snap` (regular files only, so
  the `partial/` download dir is skipped). Per snap, the revision with the
  newest mtime is the last install/refresh.
- If an older revision is still on disk (snapd keeps ~2), the line reads
  "firefox 4100 -> 4200"; otherwise "hello rev 42".
- Local revisions like `x1` are accepted. No snapd directory is silent;
  an unreadable one gives a note. 54 tests + 1 ignored.

Chose mtime over ctime so tests can set it (`File::set_modified`). Not
checked on a real Ubuntu box: worth confirming snapd doesn't preserve the
store's mtime when it moves the downloaded file into place (if it does,
ctime would be the better clock). `state.json` was skipped (root-only, big).

Next: CLI roadmap 12: `--only pkg,config` (filter by source name from
`collect_all`, error on unknown names listing the valid ones) and
`--until` (reuse `since` parsing). Keep notes only from the chosen sources.

## 2026-11-06

**CLI: `--only` and `--until`** (roadmap 12), no new dependencies.
- `sources::SOURCES` lists every source as (name, kind). `select("pkg,config")`
  matches a word against either, so `pkg` picks pkg + flatpak + snap (they
  all show as `pkg`) and `env` picks the shell source. Unknown words are an
  error that lists the valid names; the run order stays the usual one.
- `collect_all` now takes the chosen names and only runs those sources, so
  notes from unselected sources don't show up either.
- `--until WHEN` reuses `parse_since` (its error no longer says "--since";
  main prefixes the flag). Events at or after `until` are dropped after
  collecting. `--until` not later than `--since` is an error.
  56 tests + 1 ignored.

Limits: `--until` filters after `pkg::group`, so a group of package
changes is kept/dropped by its group time. Sources still read everything
after `since` (fine, logs are small).

Next: maybe an "at a glance" summary line at the end ("12 package changes,
3 configs edited"), or `/var/log/apt/history.log` to show *which command*
did an upgrade (`Commandline: apt install foo`). The latter is more useful
for "who did this" — start there.

## 2026-11-08

**apt history: which command did it**, no new dependencies. In `sources/dpkg.rs`:
- Reads `/var/log/apt/history.log{.1,}` (optional, missing is silent) into
  runs: Start-Date, End-Date, Commandline, Requested-By (uid stripped).
  Dates have two spaces between day and time; whitespace is normalized.
- A dpkg event (group) whose time is inside a run gets
  ` (apt upgrade, by majid)`, or just ` (command)` for root /
  unattended-upgrades. Commands longer than 60 chars are cut with "...".
- Runs without a Commandline are skipped; a run without End-Date
  (interrupted apt) counts as one hour long. 59 tests + 1 ignored.

Checked on this Debian box: unattended upgrades show as
`(/usr/bin/unattended-upgrade)`, which is exactly the "it changed by
itself overnight" answer.

Limits: a dpkg group is matched by its first action's time only. Plain
`dpkg -i foo.deb` has no apt run, so no command (correct). Rotated `.gz`
history is ignored, like dpkg.log.

Next: an "at a glance" summary line at the end ("12 package changes,
3 configs edited"), or check pacman.log's `[PACMAN] Running 'pacman -Syu'`
lines to do the same for Arch.

## 2026-11-09

**pacman: which command did it**, no new dependencies.
- Moved `Run`, `shorten` and `annotate` from `dpkg.rs` to `pkg.rs` so
  both sources share them (no change to the dpkg output).
- `pacman.rs` now reads `[PACMAN] Running 'pacman -Syu'` lines as runs.
  A run ends at its `[ALPM] transaction completed/failed/interrupted`
  line, or one second before the next Running line, or after an hour if
  neither comes. Package groups inside a run get ` (pacman -Syu)`.
  pacman doesn't log the user, so there is no "by ...". 61 tests + 1 ignored.

Limits: a `pacman -Syu` that had nothing to do leaves its run open for an
hour, so a pamac/PackageKit transaction (no Running line) in that hour
would be credited to it. Not checked on a real Arch box (none here).

Next: an "at a glance" summary line at the end ("12 package changes,
3 configs edited"), shown only when there is more than a screenful, or
dnf's history to show the command for Fedora too (`dnf5 history` db is
SQLite, so maybe only dnf.rpm.log / dnf.log `Command:`-style lines).

## 2026-11-10

**At-a-glance summary line**, no new dependencies.
- `timeline::summary` counts events by kind, most common first (ties by
  name): `37 changes: 26 pkg, 7 config, 2 env, 1 cron, 1 kernel`.
- Only printed (on stdout, after a blank line) when there are more than
  `SUMMARY_MIN` = 15 events; a short timeline is its own summary.
- It counts timeline lines, not packages: one grouped "upgraded a, b, c"
  line counts as 1 pkg. 62 tests + 1 ignored. Checked on this box with
  `--since 30d`.

Next: dnf's command for Fedora (look at what dnf.log / dnf5.log record
for the command line; dnf5 history is SQLite, skip that), or a `--version`
/ `--help` smoke test of argument parsing by moving it out of `main` into
a testable function.

## 2026-11-11

**Argument parsing moved to `src/cli.rs` and tested**, no new dependencies.
- `cli::parse(args) -> Result<Command, String>`, where `Command` is
  `Help`, `Version` or `Run(Options)`. `main` just matches on it.
- `--name=value` is split once up front, so both spellings go through the
  same code (removes four `starts_with` arms). Short `-s=2h` is not split.
- Small behaviour change: an empty value (`--only=`, `--since ""`) is now
  "needs a value" instead of a later, vaguer error.
- `-s`/`-u` already existed but were undocumented; now in `--help` (realigned)
  and the README. 68 tests + 1 ignored.

Next: dnf's command for Fedora (check what dnf.log / dnf5.log record for
the command line; skip the dnf5 SQLite history), or an integration test that
runs every source against a small fake `--root` tree under `target/`.

## 2026-11-12

**Integration test against a fake `--root`**, no new dependencies.
`tests/fake_root.rs` runs the real binary (`CARGO_BIN_EXE_whatchanged`)
with `TZ=UTC` against small trees built under `CARGO_TARGET_TMPDIR`:
- `every_source_shows_up`: dpkg + apt history, pacman, dnf, flatpak, snap,
  kernel, /etc and ~/.config, shell files, systemd (unit + enable link),
  crontab. Checks each shows up, with every kind of line present.
- `only_and_until_narrow_the_timeline`: `--only pkg --until` cuts the rest.
- `empty_root_...`: "Nothing changed since ..." on stdout, "Skipped:" on
  stderr, exit 0.
- `odd_and_large_logs_do_not_crash`: invalid UTF-8, a 1 MB line, broken
  timestamps, 50k old lines, broken pacman/apt history; the one good line
  still shows.
- `bad_arguments_exit_2`: bad --since/--only/--until order/unknown flag/
  missing root all exit 2 and point at --help.
68 unit + 5 integration tests + 1 ignored. All sources passed first try;
only my fixture was wrong (kernel needs `/boot/vmlinuz-VER` or a
`vmlinuz` in the modules dir, not just the modules dir).

### Before relying on it (for the first real use, ~2026-11-21)
- File-based sources (config, env, service, cron, flatpak, snap, kernel)
  use modification times. Restoring a backup, `cp -p`, rsync or a git
  checkout can set old or new times, so a file may appear or vanish
  from the timeline for reasons unrelated to a real edit.
- Package logs: only the current and `.1` rotated logs are read; `.gz`
  rotations are ignored, so `--since` far back may miss package changes.
- A unit that is both created and enabled shows two lines (`enabled` and
  `edited`). Harmless, but looks doubled.
- Without root, some dirs (e.g. `/var/spool/cron/crontabs`) can't be read;
  that's listed under "Skipped:" on stderr, not hidden.

Next: maybe merge the doubled systemd "enabled"/"edited" lines for the same
unit, or note `.gz` rotation in `--help`/README so nobody is surprised.

## 2026-11-13

**Say when rotated package logs we don't read may hold changes**, no new
dependencies.
- `pkg::unread_rotations`: for each package log (dpkg, pacman, dnf), lists
  sibling files like `dpkg.log.2.gz` / `dnf.rpm.log.3` that we don't read
  and whose mtime is at or after `--since` (gzip keeps the mtime of the
  last write, so an older mtime means nothing in the window). One
  "Skipped:" note per log dir, up to 3 names then "and N more".
- On this box `--since 60d --only pkg` now says `dpkg.log.2.gz` isn't read.
- README mentions it under the dpkg source. 69 unit + 5 integration tests.
- apt's history.log rotations are not checked: they only add the
  "(apt upgrade, by ...)" annotation, so missing them loses no events.

Next: merge the doubled systemd "enabled"/"edited" lines for one unit, or
check `--help` once more against the README before the 2026-11-21 first use.

## 2026-11-13 (second session, interactive)

**Read compressed and older rotated package logs**, still no dependencies.
- `src/gzip.rs`: a small gzip reader (header, inflate with stored, fixed
  and dynamic blocks, CRC-32 and length check, several members, an output
  size limit). Bit-at-a-time decoding like zlib's `puff`: slow next to
  zlib but rotated logs are small. Tests: fixtures for each block type,
  every truncation and every single-bit flip of them (error or same
  output, never a panic), the size limit. The ignored test
  `matches_system_gzip` compares against `gzip -dc` on the machine's own
  .gz files (`cargo test --release -- --ignored matches_system_gzip`);
  7486 files matched on this box.
- `sources::read_logs` now takes `since` and reads every rotation of a
  log written at or after it: `x.log.N`, `x.log.N.gz`, and logrotate's
  dated `x.log-20240501(.gz)`, oldest first. `.xz`/`.bz2`/`.zst` rotations
  get a "not read" note; a damaged .gz gets "can't unpack ...". If both
  `x.1` and `x.1.gz` exist only the plain one is read. Unpacking stops at
  256 MB.
- `LOGS` in dpkg/dnf/pacman (and apt history) now list only the base
  name. `pkg::unread_rotations` and its "older rotated logs are not read"
  note are gone: they are read now.
- **systemd**: a unit file written in the same minute it was enabled is
  one line, `backup.service edited and enabled`, instead of two
  (`merge_new_units`, per unit directory so labels stay right).
- `--since 99999999999999999w` said nothing useful (overflow: a panic in
  debug builds); now "'...' is too far back".
- 77 unit + 5 integration tests + 2 ignored. The fake-root test has a
  `dpkg.log.2.gz`.

For the first real use: the "Package logs: only the current and `.1`..."
caveat above no longer applies. What is left: logs that logrotate already
deleted are gone for good (Debian keeps 12 monthly dpkg logs).

Next: check `--help` once more against the README; maybe a release
(v0.2.0) before 2026-11-21, moving whatchanged-action's v1 tag after.

## 2026-11-13 (third session, interactive) — 0.2.0

**New `boot` source, a man page, version 0.2.0.** Still no dependencies.
- `src/sources/boot.rs`: `booted <kernel>` and `shut down` lines (kind
  `boot`). Reads wtmpdb's SQLite file (`/var/lib/wtmpdb/wtmp.db`, else
  `/var/log/wtmp.db`): rows with Type 1 have Login = boot time (usec),
  Logout = shutdown, RemoteHost = kernel. Also the classic `/var/log/wtmp`
  and `wtmp.1` (384-byte records; type 2 = boot, type 1 with user
  `shutdown`). Falls back to `/proc/stat` btime for the running boot.
  Records within 120 s of each other count as the same boot. Debian 13
  writes boots only to wtmp.db, which is why the SQLite reader exists.
- `src/sqlite.rs`: minimal read-only SQLite reader: file header, schema
  table, table b-tree (interior + leaf pages), overflow pages, record
  decoding. No WAL: a database in WAL mode with unsaved pages would read
  slightly stale (wtmpdb uses a rollback journal). Fixture
  `tests/data/many.db.gz` (512-byte pages, 400 rows, an overflow row) is
  unpacked with our own gunzip in the test.
- `doc/whatchanged.1`: man page; the release tarball now includes it.
  `tests/fake_root.rs::docs_cover_every_option_and_source` fails if
  `--help`, the README or the man page miss an option or a source name.
- Cargo.toml is 0.2.0. Not tagged yet: the owner decides when. To release:
  tag `v0.2.0`, run the release workflow (workflow_dispatch with the tag
  if the push doesn't trigger it), then move whatchanged-action's `v1`.
- 85 unit + 6 integration tests + 2 ignored.

The roadmap is done. What could still come, none of it needed: dnf5's
transaction history (SQLite; the reader is there now, but it needs a real
Fedora database to test against), the command for dnf changes, `.xz`/`.zst`
rotated logs.
