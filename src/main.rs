mod since;
mod sources;
mod time;
mod timeline;
mod tz;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
whatchanged - show what changed on this machine

Usage: whatchanged [--since WHEN]

  --since WHEN   start of the timeline (default: yesterday)
                 WHEN can be: today, yesterday, 30m, 2h, 3d, 1w,
                 2024-05-01, \"2024-05-01 09:00\"
  --root DIR     look at the system mounted at DIR instead of /
                 (e.g. a broken install mounted from a rescue USB)
  -h, --help     show this help
  -V, --version  show the version

Times are local, taken from $TZ or /etc/localtime.
";

fn main() -> ExitCode {
    let mut since_arg = String::from("yesterday");
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
    let since = match since::parse_since(&since_arg, time::now(), &tz) {
        Ok(t) => t,
        Err(e) => return fail(&e),
    };

    let mut events = Vec::new();
    let mut notes: Vec<String> = tz_note.into_iter().collect();
    for (_name, result) in sources::collect_all(&root, since, &tz) {
        events.extend(result.events);
        notes.extend(result.notes);
    }

    if events.is_empty() {
        println!("Nothing changed since {since_arg} (that I could see).");
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
