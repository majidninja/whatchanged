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
```

## Status

Early days. Right now it reads the **dpkg log** (Debian, Ubuntu, Mint, ...).
More sources are coming: dnf, pacman, flatpak, files under `/etc` and
`~/.config`, systemd units, crontabs, shell startup files, kernels.

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
