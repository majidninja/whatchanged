//! Command-line argument parsing, kept apart from `main` so it can be tested.

use std::path::PathBuf;

/// What the user asked for.
#[derive(Debug, PartialEq)]
pub enum Command {
    Help,
    Version,
    Run(Options),
}

#[derive(Debug, PartialEq)]
pub struct Options {
    pub since: String,
    pub until: Option<String>,
    pub only: Option<String>,
    pub root: PathBuf,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            since: String::from("yesterday"),
            until: None,
            only: None,
            root: PathBuf::from("/"),
        }
    }
}

/// Parses the arguments (without the program name). `--help` and
/// `--version` win as soon as they are seen, like most tools.
/// Errors are messages ready to print after "whatchanged: ".
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Command, String> {
    let mut o = Options::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        // Split "--name=value" so both spellings take the same path.
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n.to_string(), Some(v.to_string())),
            _ => (arg.clone(), None),
        };
        let hint = match name.as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            "--since" | "-s" => "a value, e.g. --since 2h",
            "--until" | "-u" => "a value, e.g. --until today",
            "--only" => "a list, e.g. --only pkg,config",
            "--root" => "a directory, e.g. --root /mnt",
            _ => return Err(format!("unknown argument '{arg}'")),
        };
        let value = match inline.or_else(|| args.next()) {
            Some(v) if !v.is_empty() => v,
            _ => return Err(format!("{name} needs {hint}")),
        };
        match name.as_str() {
            "--since" | "-s" => o.since = value,
            "--until" | "-u" => o.until = Some(value),
            "--only" => o.only = Some(value),
            _ => o.root = PathBuf::from(value),
        }
    }
    Ok(Command::Run(o))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<Command, String> {
        parse(args.iter().map(|s| s.to_string()))
    }

    fn run(args: &[&str]) -> Options {
        match p(args) {
            Ok(Command::Run(o)) => o,
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn defaults() {
        assert_eq!(run(&[]), Options::default());
        assert_eq!(run(&[]).since, "yesterday");
        assert_eq!(run(&[]).root, PathBuf::from("/"));
    }

    #[test]
    fn help_and_version() {
        assert_eq!(p(&["--help"]), Ok(Command::Help));
        assert_eq!(p(&["-h"]), Ok(Command::Help));
        assert_eq!(p(&["--version"]), Ok(Command::Version));
        assert_eq!(p(&["-V"]), Ok(Command::Version));
        // Help wins even after other options or before a bad one.
        assert_eq!(p(&["--since", "2h", "--help"]), Ok(Command::Help));
        assert_eq!(p(&["-h", "--bogus"]), Ok(Command::Help));
    }

    #[test]
    fn separate_and_equals_forms() {
        let o = run(&["--since", "2h", "--until=today", "--only", "pkg,config"]);
        assert_eq!(o.since, "2h");
        assert_eq!(o.until.as_deref(), Some("today"));
        assert_eq!(o.only.as_deref(), Some("pkg,config"));
        let o = run(&["--since=2024-05-01 09:00", "--root=/mnt"]);
        assert_eq!(o.since, "2024-05-01 09:00");
        assert_eq!(o.root, PathBuf::from("/mnt"));
        let o = run(&["-s", "3d", "-u", "1d"]);
        assert_eq!((o.since.as_str(), o.until.as_deref()), ("3d", Some("1d")));
    }

    #[test]
    fn later_value_wins() {
        assert_eq!(run(&["--since", "2h", "--since", "3d"]).since, "3d");
    }

    #[test]
    fn missing_values() {
        assert_eq!(
            p(&["--since"]),
            Err("--since needs a value, e.g. --since 2h".into())
        );
        assert_eq!(
            p(&["--root"]),
            Err("--root needs a directory, e.g. --root /mnt".into())
        );
        assert_eq!(
            p(&["--only="]),
            Err("--only needs a list, e.g. --only pkg,config".into())
        );
    }

    #[test]
    fn unknown_arguments() {
        assert_eq!(p(&["--bogus"]), Err("unknown argument '--bogus'".into()));
        assert_eq!(
            p(&["--bogus=1"]),
            Err("unknown argument '--bogus=1'".into())
        );
        assert_eq!(
            p(&["yesterday"]),
            Err("unknown argument 'yesterday'".into())
        );
        // A short option with '=' is not split.
        assert_eq!(p(&["-s=2h"]), Err("unknown argument '-s=2h'".into()));
    }
}
