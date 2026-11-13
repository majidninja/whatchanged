mod cli;
mod gzip;
mod since;
mod sources;
mod sqlite;
mod time;
mod timeline;
mod tz;

use std::process::ExitCode;

const USAGE: &str = "\
whatchanged - show what changed on this machine

Usage: whatchanged [--since WHEN] [--until WHEN] [--only LIST] [--root DIR]

  -s, --since WHEN  start of the timeline (default: yesterday)
                    WHEN can be: today, yesterday, 30m, 2h, 3d, 1w,
                    2024-05-01, \"2024-05-01 09:00\"
  -u, --until WHEN  end of the timeline (default: now), same forms
      --only LIST   only these sources, comma-separated:
                    pkg, flatpak, snap, kernel, boot, config,
                    shell (or env), service, cron; pkg includes
                    flatpak and snap
      --root DIR    look at the system mounted at DIR instead of /
                    (e.g. a broken install mounted from a rescue USB)
  -h, --help        show this help
  -V, --version     show the version

Times are local, taken from $TZ or /etc/localtime.
";

fn main() -> ExitCode {
    let opts = match cli::parse(std::env::args().skip(1)) {
        Ok(cli::Command::Help) => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Ok(cli::Command::Version) => {
            println!("whatchanged {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Ok(cli::Command::Run(o)) => o,
        Err(e) => return fail(&e),
    };
    let cli::Options {
        since: since_arg,
        until: until_arg,
        only: only_arg,
        root,
    } = opts;

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
        if let Some(line) = timeline::summary(&events) {
            println!();
            println!("{line}");
        }
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
