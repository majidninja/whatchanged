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

- the **dpkg log** (Debian, Ubuntu, Mint, ...)
- the **pacman log** (Arch, Manjaro, EndeavourOS, ...): installs, upgrades,
  downgrades and removals from `/var/log/pacman.log`. If you don't use
  pacman, it stays quiet.
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

More sources are coming: dnf and flatpak.

Times are shown in your local time zone, the same one `date` uses: it comes
from `$TZ` if that is set, otherwise from `/etc/localtime`. To see the timeline
in another zone, set it for one run, e.g. `TZ=UTC whatchanged --since 2h`.

## Install

You need a Rust toolchain ([rustup.rs](https://rustup.rs)).

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

You don't need root. If something can't be read (a missing log, no
permission), `whatchanged` keeps going and lists what it skipped at the end,
on stderr, so you know the picture may be incomplete.

## Design goals

- Plain text output that stays stable, so it's easy to grep and diff.
- Few dependencies (currently none besides Rust's standard library).
- Every data source is a small, separate module that fails gracefully.

## License

MIT
