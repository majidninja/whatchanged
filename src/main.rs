mod since;
mod sources;
mod time;
mod timeline;
mod tz;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
whatchanged - show what changed on this machine

Usage: whatchanged [--since WHEN] [--until WHEN] [--only LIST] [--root DIR]

  --since WHEN   start of the timeline (default: yesterday)
                 WHEN can be: today, yesterday, 30m, 2h, 3d, 1w,
                 2024-05-01, \"2024-05-01 09:00\"
  --until WHEN   end of the timeline (default: now), same forms
  --only LIST    only these sources, comma-separated:
                 pkg, flatpak, snap, kernel, config, shell (or env),
                 service, cron; pkg includes flatpak and snap
  --root DIR     look at the system mounted at DIR instead of /
                 (e.g. a broken install mounted from a rescue USB)
  -h, --help     show this help
  -V, --version  show the version

Times are local, taken from $TZ or /etc/localtime.
";

fn main() -> ExitCode {
    let mut since_arg = String::from("yesterday");
    let mut until_arg: Option<String> = None;
    let mut only_arg: Option<String> = None;
    let mut root = PathBuf::from("/");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!("whatchanged {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            "--since" | "-s" => match args.next() {
                Some(v) => since_arg = v,
                None => return fail("--since needs a value, e.g. --since 2h"),
            },
            _ if arg.starts_with("--since=") => since_arg = arg["--since=".len()..].to_string(),
            "--until" | "-u" => match args.next() {
                Some(v) => until_arg = Some(v),
                None => return fail("--until needs a value, e.g. --until today"),
            },
            _ if arg.starts_with("--until=") => {
                until_arg = Some(arg["--until=".len()..].to_string())
            }
            "--only" => match args.next() {
                Some(v) => only_arg = Some(v),
                None => return fail("--only needs a list, e.g. --only pkg,config"),
            },
            _ if arg.starts_with("--only=") => only_arg = Some(arg["--only=".len()..].to_string()),
            "--root" => match args.next() {
                Some(v) => root = PathBuf::from(v),
                None => return fail("--root needs a directory, e.g. --root /mnt"),
            },
            _ if arg.starts_with("--root=") => root = PathBuf::from(&arg["--root=".len()..]),
            _ => return fail(&format!("unknown argument '{arg}'")),
        }
    }

    if !root.is_dir() {
        return fail(&format!("--root: {} is not a directory", root.display()));
    }

    let (tz, tz_note) = tz::Tz::local(&root);
    let now = time::now();
    let since = match since::parse_since(&since_arg, now, &tz) {
        Ok(t) => t,
        Err(e) => return fail(&format!("--since: {e}")),
    };
    let until = match &until_arg {
        None => None,
        Some(a) => match since::parse_since(a, now, &tz) {
            Ok(t) if t <= since => return fail("--until must be later than --since"),
            Ok(t) => Some(t),
            Err(e) => return fail(&format!("--until: {e}")),
        },
    };
    let names = match &only_arg {
        None => sources::all_names(),
        Some(a) => match sources::select(a) {
            Ok(n) => n,
            Err(e) => return fail(&format!("--only: {e}")),
        },
    };

    let mut events = Vec::new();
    let mut notes: Vec<String> = tz_note.into_iter().collect();
    for (_name, result) in sources::collect_all(&root, since, &tz, &names) {
        events.extend(result.events);
        notes.extend(result.notes);
    }
    if let Some(until) = until {
        events.retain(|e| e.time < until);
    }

    if events.is_empty() {
        match &until_arg {
            Some(u) => println!("Nothing changed between {since_arg} and {u} (that I could see)."),
            None => println!("Nothing changed since {since_arg} (that I could see)."),
        }
    } else {
        print!("{}", timeline::render(&events, &tz));
    }
    if !notes.is_empty() {
        eprintln!();
        eprintln!("Skipped:");
        for n in notes {
            eprintln!("  {n}");
        }
    }
    ExitCode::SUCCESS
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("whatchanged: {msg}");
    eprintln!("Try 'whatchanged --help'.");
    ExitCode::from(2)
}
