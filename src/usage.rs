//! Usage errors: every rejected invocation prints `error: <summary>`, a blank
//! line and the help of the deepest matched subcommand, then exits 2.

use std::ffi::OsString;
use std::io::Write;

use crate::cli::{build_command, Binary};

/// A usage error raised after parsing; `execute` prints it with the help of
/// the command at `command_path`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageError {
    /// Subcommand names below the binary root, e.g. `["serve","config","set"]`.
    pub command_path: Vec<String>,
    /// Human-readable summary.
    pub message: String,
}

impl std::fmt::Display for UsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for UsageError {}

/// The clap message without the `error: ` prefix, `Usage:` paragraph and
/// `For more information` tail; continuation lines are kept.
pub fn clap_error_summary(e: &clap::Error) -> String {
    // `StyledStr`'s Display never emits ANSI escapes.
    let rendered = e.render().to_string();
    let head = rendered.split("\n\n").next().unwrap_or_default().trim_end();
    let head = head.strip_prefix("error: ").unwrap_or(head);
    head.lines()
        .take_while(|l| !l.starts_with("Usage:") && !l.starts_with("For more information"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Write `error: {message}`, a blank line and the short help (the `-h` text)
/// of the subcommand at `path` to `w`; returns the exit status 2.
pub fn print_usage_error_to(
    w: &mut dyn Write,
    binary: Binary,
    path: &[String],
    message: &str,
) -> i32 {
    let root = build_command(binary);
    let mut cmd = path
        .iter()
        .try_fold(&root, |c, name| c.find_subcommand(name))
        .unwrap_or(&root)
        .clone();
    let help = cmd.render_help();
    // A closed stderr must not change the exit status.
    let _ = write!(w, "error: {message}\n\n{help}");
    2
}

/// [`print_usage_error_to`] on stderr.
pub fn print_usage_error(binary: Binary, path: &[String], message: &str) -> i32 {
    print_usage_error_to(&mut std::io::stderr().lock(), binary, path, message)
}

/// Whether `cmd` (or an ancestor) declares a value-taking arg for the token.
fn takes_value(chain: &[&clap::Command], long: Option<&str>, short: Option<char>) -> bool {
    chain.iter().any(|c| {
        c.get_arguments().any(|a| {
            a.get_action().takes_values()
                && (long.is_some() && a.get_long() == long
                    || short.is_some() && a.get_short() == short)
        })
    })
}

/// Names of the subcommands below `root` that `argv` selected, ignoring flags
/// and their values; stops at the first token that is no subcommand.
pub fn deepest_command_path(root: &clap::Command, argv: &[OsString]) -> Vec<String> {
    let mut chain: Vec<&clap::Command> = vec![root];
    let mut path = Vec::new();
    let mut tokens = argv.iter().skip(1).map(|t| t.to_string_lossy());
    while let Some(token) = tokens.next() {
        if token == "--" {
            break;
        }
        if let Some(long) = token.strip_prefix("--") {
            if !long.contains('=') && takes_value(&chain, Some(long), None) {
                tokens.next();
            }
        } else if let Some(shorts) = token.strip_prefix('-').filter(|s| !s.is_empty()) {
            let chars: Vec<char> = shorts.chars().collect();
            if let Some(i) = chars
                .iter()
                .position(|c| takes_value(&chain, None, Some(*c)))
            {
                if i + 1 == chars.len() {
                    tokens.next();
                }
            }
        } else {
            let current = chain[chain.len() - 1];
            match current.find_subcommand(&*token) {
                Some(sub) => {
                    path.push(token.into_owned());
                    chain.push(sub);
                }
                None => break,
            }
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::parse_cli;

    fn argv(a: &[&str]) -> Vec<OsString> {
        a.iter().map(OsString::from).collect()
    }

    fn summary(binary: Binary, a: &[&str]) -> String {
        clap_error_summary(&parse_cli(binary, argv(a)).unwrap_err())
    }

    fn path(binary: Binary, a: &[&str]) -> Vec<String> {
        deepest_command_path(&build_command(binary), &argv(a))
    }

    fn strs(p: &[&str]) -> Vec<String> {
        p.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn summary_keeps_continuation_lines() {
        let s = summary(Binary::Serve, &["its", "-n", "web"]);
        assert!(s.contains("the following required arguments were not provided"));
        assert!(s.contains("--target"));
        assert!(!s.contains("Usage:") && !s.contains("For more information"));
        assert!(!s.starts_with("error:"));
        let s = summary(
            Binary::Serve,
            &["its", "-n", "web", "-t", "1", "-p", "sctp"],
        );
        assert!(s.contains("possible values: tcp, udp"), "{s}");
    }

    #[test]
    fn renders_error_then_help() {
        let mut buf = Vec::new();
        let code = print_usage_error_to(
            &mut buf,
            Binary::AllInOne,
            &strs(&["serve", "config", "set"]),
            "x",
        );
        let text = String::from_utf8(buf).unwrap();
        assert_eq!(code, 2);
        assert!(text.starts_with("error: x\n\n"), "{text}");
        assert!(
            text.contains("Usage: iroh-tunnel serve config set"),
            "{text}"
        );
    }

    #[test]
    fn standalone_usage_error_shows_standalone_help() {
        let mut buf = Vec::new();
        print_usage_error_to(&mut buf, Binary::Serve, &strs(&["config", "set"]), "x");
        let text = String::from_utf8(buf).unwrap();
        assert!(
            text.contains("Usage: iroh-tunnel-serve config set"),
            "{text}"
        );
    }

    #[test]
    fn deepest_command_skips_global_flags() {
        let a = Binary::AllInOne;
        assert_eq!(
            path(
                a,
                &[
                    "it",
                    "-v",
                    "--color=never",
                    "serve",
                    "config",
                    "set",
                    "--bogus"
                ]
            ),
            strs(&["serve", "config", "set"])
        );
        assert_eq!(
            path(a, &["it", "--color", "never", "serve", "config", "bogus"]),
            strs(&["serve", "config"])
        );
        assert_eq!(path(a, &["it", "serve", "-n", "config"]), strs(&["serve"]));
        assert_eq!(path(Binary::Serve, &["its", "--", "config"]), strs(&[]));
        assert_eq!(
            path(a, &["it", "serve", "--color", "never", "config", "bogus"]),
            strs(&["serve", "config"])
        );
    }
}
