//! Issue #82 CLI-HELP-01: actual role-process help and rejected options.

use std::process::{Command, Output};

struct RoleForm {
    binary: &'static str,
    prefix: &'static [&'static str],
    serve: bool,
}

const FORMS: [RoleForm; 4] = [
    RoleForm {
        binary: env!("CARGO_BIN_EXE_iroh-tunnel"),
        prefix: &["serve"],
        serve: true,
    },
    RoleForm {
        binary: env!("CARGO_BIN_EXE_iroh-tunnel"),
        prefix: &["access"],
        serve: false,
    },
    RoleForm {
        binary: env!("CARGO_BIN_EXE_iroh-tunnel-serve"),
        prefix: &[],
        serve: true,
    },
    RoleForm {
        binary: env!("CARGO_BIN_EXE_iroh-tunnel-access"),
        prefix: &[],
        serve: false,
    },
];

fn invoke(form: &RoleForm, args: &[&str]) -> Output {
    let home = tempfile::tempdir().unwrap();
    Command::new(form.binary)
        .args(form.prefix)
        .args(args)
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path())
        .env("IROH_TUNNEL_TEST_CONFIG_DIR", home.path())
        .env("IROH_TUNNEL_STATE_DIR", home.path())
        .output()
        .unwrap()
}

#[test]
fn role_help_documents_existing_options_and_tcp_default() {
    for form in &FORMS {
        let output = invoke(form, &["--help"]);
        let help = String::from_utf8(output.stdout).unwrap();
        assert!(
            output.status.success(),
            "{} {:?}: {help}",
            form.binary,
            form.prefix
        );
        for option in [
            "-n, --name",
            "-p, --protocol",
            "[default: tcp]",
            "--relay-url",
            "--help",
        ] {
            assert!(
                help.contains(option),
                "{} {:?}: missing {option}: {help}",
                form.binary,
                form.prefix
            );
        }
        let options: &[&str] = if form.serve {
            &["-t, --target"]
        } else {
            &["-l, --listen", "-N, --node-id"]
        };
        for option in options {
            assert!(
                help.contains(option),
                "{} {:?}: missing {option}: {help}",
                form.binary,
                form.prefix
            );
        }
    }
}

#[test]
fn role_specific_options_are_rejected_with_guidance() {
    for form in &FORMS {
        let options: &[&str] = if form.serve {
            &["--listen", "-l", "--node-id", "-N"]
        } else {
            &["--target", "-t"]
        };
        for option in options {
            let output = invoke(form, &[option, "invalid"]);
            let error = String::from_utf8(output.stderr).unwrap();
            assert!(
                !output.status.success(),
                "{} {:?} {option}",
                form.binary,
                form.prefix
            );
            assert!(
                error.contains(option) && error.contains("--help"),
                "{} {:?}: {error}",
                form.binary,
                form.prefix
            );
        }
    }
}

#[test]
fn quick_options_conflict_with_configured_commands() {
    for form in &FORMS {
        let mut flags = vec![
            ("--name", "echo"),
            ("-n", "echo"),
            ("--protocol", "tcp"),
            ("-p", "tcp"),
            ("--relay-url", "https://localhost:1"),
        ];
        if form.serve {
            flags.extend([("--target", "127.0.0.1:12345"), ("-t", "127.0.0.1:12345")]);
        } else {
            flags.extend([
                ("--listen", "127.0.0.1:12345"),
                ("-l", "127.0.0.1:12345"),
                ("--node-id", "invalid"),
                ("-N", "invalid"),
            ]);
        }
        for (flag, value) in flags {
            let output = invoke(form, &[flag, value, "config", "path"]);
            let error = String::from_utf8(output.stderr).unwrap();
            assert!(
                !output.status.success(),
                "{} {:?} {flag}",
                form.binary,
                form.prefix
            );
            assert!(
                error.contains("cannot be used with")
                    && error.contains("config")
                    && error.contains("--help"),
                "{} {:?}: {error}",
                form.binary,
                form.prefix
            );
        }
    }
}
