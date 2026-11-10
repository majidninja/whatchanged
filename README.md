# whatchanged

Something on your Linux machine broke, and it worked yesterday. What changed?

`whatchanged` answers that by putting everything it can find on one timeline:
package upgrades, edited config files, enabled services, and so on. It only
reads what's already on disk. No daemon, no network, nothing is modified.

```
$ whatchanged --since yesterday
2026-10-22
  06:16  pkg     upgraded  nginx 1.26.3-3+deb13u9 -> 1.26.3-3+deb13u10, ca-certificates 20250419 -> 20250419+deb13u1
  18:58  pkg     installed mesa-vulkan-drivers 25.0.7-2+deb13u1
  18:59  kernel  installed 6.12.113+deb13-amd64 (not booted yet)
```

## Status

Early days. Right now it reads:

- the **dpkg log** (Debian, Ubuntu, Mint, ...): installs, upgrades,
  removals and purges from `/var/log/dpkg.log` and `/var/log/dpkg.log.1`.
  If apt's `/var/log/apt/history.log` is there too, each line also says
  which command made the change and who ran it, so you can tell your own
  `apt upgrade` from an automatic one:
  `installed gh 2.46.0-3 (apt install gh, by majid)` or
  `upgraded  apt 3.0.3 -> 3.0.4 (/usr/bin/unattended-upgrade)`.
- the **pacman log** (Arch, Manjaro, EndeavourOS, ...): installs, upgrades,
  downgrades and removals from `/var/log/pacman.log`, with the command
  that made them when pacman logged one:
  `upgraded  mesa 1:24.1-1 -> 1:24.2-1 (pacman -Syu)`. Graphical tools
  like pamac don't log a command, so their changes show without one.
- the **dnf log** (Fedora, RHEL, CentOS Stream, ...): the same, from
  `/var/log/dnf.rpm.log`. Versions are shown as `version-release`, e.g.
  `kernel-core 6.12.4-200.fc40 -> 6.12.5-200.fc40`. Fedora 41 and newer
  use dnf5, which only keeps a database whatchanged can't read. For dnf5
  you get one line with the time of the last package change, and a hint
  to run `dnf5 history info last` for the details.

  It only mentions a missing package log if it found none at all, so
  an Arch machine won't complain about dpkg, and vice versa.
- **flatpak apps and runtimes**, system-wide and per user, as `pkg` lines like
  `flatpak   org.mozilla.firefox, org.gnome.Platform//47`. Flatpak keeps no
  log, so this is the time each app was last installed or updated (it can't
  tell which), and removed apps don't show up. Per-user installs are marked
  `(user)`.
- **snaps**, from the files in `/var/lib/snapd/snaps`, as `pkg` lines like
  `snap      firefox 4100 -> 4200, hello rev 42`. snapd keeps an old
  revision or two after a refresh, so when one is still there you see which
  revision it came from. Removed and reverted snaps don't show up.
- **installed kernels** (any distro). It also tells you which one is running,
  and marks kernels installed since the last boot as "not booted yet", which
  is a common answer to "why is it different after the reboot?"
- **shell startup files**: `~/.bashrc`, `~/.profile`, `~/.zshrc`, fish's
  `config.fish`, `/etc/profile`, `/etc/profile.d/*`, `/etc/environment`, ...
  for every user in `/home` (and `/root`, if you're allowed to look there).
  These show up as `env` lines. A bad line in one of them is a common reason
  for a wrong `PATH` or a terminal that suddenly behaves differently.

- **config files under `/etc` and `~/.config`**, shown as `config` lines.
  `~/.config` is checked for `/root` and every user in `/home`, two folders
  deep. Files edited in the same minute are put on one line. Files that get
  rewritten on their own are left out: `/etc/mtab`, `/etc/ld.so.cache`,
  `/etc/resolv.conf`, browser and chat app profiles (Chromium, Chrome, Code,
  Discord, Slack, ...), caches, locks and editor backups (`foo~`, `.swp`).
  Symlinks are not followed.

- **systemd units**, shown as `service` lines: `docker.service enabled`,
  `cups.service masked`, or a unit file or override (what `systemctl edit`
  writes) edited. It looks in `/etc/systemd/system`, `/etc/systemd/user` and
  each user's `~/.config/systemd/user`; user units are marked like
  `backup.timer (user alice)`. Disabling a unit deletes a link and leaves
  nothing behind to date, so disables are not shown.

- **crontabs**, shown as `cron` lines: `/etc/crontab`, `/etc/cron.d/*`, the
  scripts in `/etc/cron.daily` and friends, and each user's own crontab (what
  `crontab -e` writes), shown as `crontab of alice edited`. User crontabs are
  in `/var/spool/cron`, which normally only root can read; without root
  you'll get a note saying they were skipped. Removed crontabs are not shown.

Times are shown in your local time zone, the same one `date` uses: it comes
from `$TZ` if that is set, otherwise from `/etc/localtime`. To see the timeline
in another zone, set it for one run, e.g. `TZ=UTC whatchanged --since 2h`.

## Install

Prebuilt static binaries for x86_64 and aarch64 Linux are on the
[releases page](https://github.com/majidninja/whatchanged/releases). They run
on any distro:

```
arch=$(uname -m)   # x86_64 or aarch64
tag=$(curl -s https://api.github.com/repos/majidninja/whatchanged/releases/latest | sed -n 's/.*"tag_name": "\(.*\)".*/\1/p')
curl -LO "https://github.com/majidninja/whatchanged/releases/download/$tag/whatchanged-$tag-$arch-unknown-linux-musl.tar.gz"
tar -xzf whatchanged-*.tar.gz
sudo install whatchanged-*/whatchanged /usr/local/bin/
```

Or build it from source with a Rust toolchain ([rustup.rs](https://rustup.rs)):

```
git clone https://github.com/majidninja/whatchanged
cd whatchanged
cargo install --path .
```

## Usage

```
whatchanged                      # since the start of yesterday
whatchanged --since 2h           # last two hours
whatchanged --since 3d           # last three days (also: 30m, 1w)
whatchanged --since today
whatchanged --since 2024-05-01
whatchanged --since "2024-05-01 09:00"
```

To look at a window in the past, add `--until` (same forms as `--since`).
To see only some kinds of change, use `--only` with a comma-separated list:

```
whatchanged --since 2024-05-01 --until 2024-05-03   # May 1st and 2nd only
whatchanged --since 1w --only pkg                   # packages (incl. flatpak, snap)
whatchanged --only config,service,env
```

The names for `--only` are `pkg`, `flatpak`, `snap`, `kernel`, `config`,
`shell` (or `env`), `service` and `cron`. Mistype one and it tells you the
valid names.

If the machine won't boot at all, start a rescue/live USB, mount the broken
system somewhere, and point `whatchanged` at it:

```
sudo mount /dev/sda2 /mnt
whatchanged --root /mnt --since 3d
```

With `--root`, times are shown in the mounted system's time zone (from its
`/etc/localtime`), not the rescue system's.

When a big `apt install` touches dozens of packages, the first few are listed
and the rest are summarised as "and N more".

When the timeline is long (more than 15 lines), one last line counts them by
kind, so you can see at a glance where most of the change happened:

```
37 changes: 26 pkg, 7 config, 2 env, 1 cron, 1 kernel
```

You don't need root. If something can't be read (a missing log, no
permission), `whatchanged` keeps going and lists what it skipped at the end,
on stderr, so you know the picture may be incomplete.

## Design goals

- Plain text output that stays stable, so it's easy to grep and diff.
- Few dependencies (currently none besides Rust's standard library).
- Every data source is a small, separate module that fails gracefully.

## License

MIT
