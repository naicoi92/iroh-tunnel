//! #82: independently packaged role binaries, aliases, consistent options.
//!
//! Each case copies ONE executable into its own empty directory (plus its
//! alias link) and runs it with `PATH` restricted to that directory, so a
//! role cannot silently depend on another installed binary. The `test-utils`
//! feature disables relay TLS verification for these binaries: use only the
//! isolated in-process relay, never public relays or real secrets.
//!
//! Acceptance map:
//! - 82.AC1 / CL1 (CLI-INDEPENDENT-01, CLI-ALIAS-01): `it serve`/`it access`,
//!   `its`/`ita` and the full standalone names forward real TCP bytes.
//! - 82.CL2 (CLI-SHORT-01, IDENTITY-KEY-01): long and short forms, TCP
//!   default, and `-k/--key-file` restart identity for every role form.
//! - 82.AC2 / CL2 (CLI-HELP-01): help lists options, aliases and defaults;
//!   parsers have no conflicting short options (unit test in `src/cli.rs`).
//! - 82.CL3: role-specific invalid options and key-file diagnostics.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const ALL_IN_ONE: &str = env!("CARGO_BIN_EXE_iroh-tunnel");
const SERVE: &str = env!("CARGO_BIN_EXE_iroh-tunnel-serve");
const ACCESS: &str = env!("CARGO_BIN_EXE_iroh-tunnel-access");

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// One executable installed alone in `dir/bin`, reachable by its real name
/// and by its alias (a symlink on Unix, a copy on Windows).
struct Installed {
    root: tempfile::TempDir,
    real: PathBuf,
    alias: PathBuf,
}

impl Installed {
    fn new(source: &str, real_name: &str, alias_name: &str) -> Result<Self> {
        let root = tempfile::tempdir()?;
        let bin = root.path().join("bin");
        std::fs::create_dir(&bin)?;
        let suffix = std::env::consts::EXE_SUFFIX;
        let real = bin.join(format!("{real_name}{suffix}"));
        let alias = bin.join(format!("{alias_name}{suffix}"));
        std::fs::copy(source, &real)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &alias)?;
        #[cfg(not(unix))]
        std::fs::copy(&real, &alias)?;
        let entries: Vec<_> = std::fs::read_dir(&bin)?
            .map(|e| e.map(|e| e.file_name()))
            .collect::<std::io::Result<_>>()?;
        eprintln!("installed {} alone: {entries:?}", bin.display());
        Ok(Self { root, real, alias })
    }

    fn home(&self) -> PathBuf {
        self.root.path().join("home")
    }

    /// Command for the alias, with `PATH` holding only this install.
    fn command(&self, use_alias: bool, state: &Path) -> Result<Command> {
        let exe = if use_alias { &self.alias } else { &self.real };
        let home = self.home();
        std::fs::create_dir_all(&home)?;
        let mut command = Command::new(exe);
        command
            .env_clear()
            .env("PATH", exe.parent().context("bin dir")?)
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", &home)
            .env("IROH_TUNNEL_TEST_CONFIG_DIR", &home)
            .env("IROH_TUNNEL_STATE_DIR", state)
            .env("RUST_LOG", "iroh_tunnel=info");
        #[cfg(windows)]
        for key in ["SystemRoot", "LOCALAPPDATA", "APPDATA", "USERPROFILE"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        Ok(command)
    }
}

/// How one role is invoked: which install, alias or real name, and the
/// role prefix (`serve`/`access` for the all-in-one, nothing standalone).
struct RoleForm<'a> {
    install: &'a Installed,
    alias: bool,
    prefix: Option<&'static str>,
}

impl RoleForm<'_> {
    fn spawn(&self, state: &Path, args: &[&str], log: &Path) -> Result<Process> {
        let mut command = self.install.command(self.alias, state)?;
        command.args(self.prefix).args(args);
        eprintln!(
            "spawn {:?} {:?} {args:?}",
            command.get_program(),
            self.prefix
        );
        Ok(Process(
            command
                .stdout(std::fs::File::create(log)?)
                .stderr(Stdio::null())
                .spawn()?,
        ))
    }

    fn output(&self, args: &[&str]) -> Result<Output> {
        let state = tempfile::tempdir()?;
        let mut command = self.install.command(self.alias, state.path())?;
        Ok(command.args(self.prefix).args(args).output()?)
    }
}

async fn wait_status(path: &Path, child: &mut Process) -> Result<serde_json::Value> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(data) = std::fs::read(path) {
            if let Ok(json) = serde_json::from_slice(&data) {
                return Ok(json);
            }
        }
        if let Some(exit) = child.0.try_wait()? {
            anyhow::bail!("role exited: {exit}");
        }
        anyhow::ensure!(Instant::now() < deadline, "status timeout");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn node_id(status: &serde_json::Value) -> Result<String> {
    Ok(status["node_id"].as_str().context("node id")?.to_owned())
}

/// Start Serve and return its public identity.
async fn start_serve(
    form: &RoleForm<'_>,
    state: &Path,
    short: bool,
    target: &str,
    relay: &str,
    key: Option<&Path>,
) -> Result<(Process, String)> {
    let key = key.map(|k| k.to_str().expect("utf-8 temp path"));
    let mut args = if short {
        vec!["-n", "echo", "-t", target, "-p", "tcp"]
    } else {
        // Protocol omitted: TCP default.
        vec!["--name", "echo", "--target", target]
    };
    args.extend(["--relay-url", relay]);
    if let Some(key) = key {
        args.extend([if short { "-k" } else { "--key-file" }, key]);
    }
    let mut process = form.spawn(state, &args, &state.join("serve.log"))?;
    let status = wait_status(&state.join("serve-status.json"), &mut process).await?;
    Ok((process, node_id(&status)?))
}

/// Start Access on a free loopback port; returns the process, its public
/// identity and listener address once the post-bind log line appears.
async fn start_access(
    form: &RoleForm<'_>,
    state: &Path,
    short: bool,
    node: &str,
    relay: &str,
    key: Option<&Path>,
) -> Result<(Process, String, std::net::SocketAddr)> {
    let key = key.map(|k| k.to_str().expect("utf-8 temp path"));
    for attempt in 0..8 {
        let listen = TcpListener::bind("127.0.0.1:0").await?.local_addr()?;
        let address = listen.to_string();
        let mut args = if short {
            vec!["-n", "echo", "-l", &address, "-N", node, "-p", "tcp"]
        } else {
            vec!["--name", "echo", "--listen", &address, "--node-id", node]
        };
        args.extend(["--relay-url", relay]);
        if let Some(key) = key {
            args.extend([if short { "-k" } else { "--key-file" }, key]);
        }
        let log_path = state.join(format!("access-{attempt}.log"));
        let mut process = form.spawn(state, &args, &log_path)?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let log = std::fs::read_to_string(&log_path)?;
            if log
                .lines()
                .any(|l| l.contains("listening for local clients") && l.contains(&address))
            {
                let status = wait_status(&state.join("access-status.json"), &mut process).await?;
                return Ok((process, node_id(&status)?, listen));
            }
            if log.contains("failed to bind") {
                break;
            }
            if let Some(exit) = process.0.try_wait()? {
                anyhow::bail!("access exited: {exit}; {log}");
            }
            anyhow::ensure!(Instant::now() < deadline, "access listener timeout: {log}");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    anyhow::bail!("access listener address in use after 8 attempts")
}

/// Forward one request/response through `serve` and `access` forms.
async fn roundtrip(serve: &RoleForm<'_>, access: &RoleForm<'_>, short: bool) -> Result<()> {
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let relay = relay.to_string();
    let target = TcpListener::bind("127.0.0.1:0").await?;
    let target_addr = target.local_addr()?.to_string();
    let fixture = async {
        let (mut stream, _) = target.accept().await?;
        let mut request = [0u8; 7];
        stream.read_exact(&mut request).await?;
        anyhow::ensure!(&request == b"request", "wrong target request");
        stream.write_all(b"response").await?;
        Ok::<_, anyhow::Error>(())
    };
    let session = async {
        let serve_state = tempfile::tempdir()?;
        let access_state = tempfile::tempdir()?;
        let (_serve, node) =
            start_serve(serve, serve_state.path(), short, &target_addr, &relay, None).await?;
        let (_access, _, listen) =
            start_access(access, access_state.path(), short, &node, &relay, None).await?;
        let mut client = TcpStream::connect(listen).await?;
        client.write_all(b"request").await?;
        let mut response = [0u8; 8];
        tokio::time::timeout(Duration::from_secs(30), client.read_exact(&mut response)).await??;
        anyhow::ensure!(&response == b"response", "wrong client response");
        eprintln!("forwarded short={short}: exact request and response");
        Ok::<_, anyhow::Error>(())
    };
    tokio::time::timeout(Duration::from_secs(120), async {
        tokio::try_join!(fixture, session)?;
        Ok::<_, anyhow::Error>(())
    })
    .await?
}

fn installs() -> Result<(Installed, Installed, Installed)> {
    Ok((
        Installed::new(ALL_IN_ONE, "iroh-tunnel", "it")?,
        Installed::new(SERVE, "iroh-tunnel-serve", "its")?,
        Installed::new(ACCESS, "iroh-tunnel-access", "ita")?,
    ))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn all_in_one_alias_runs_both_roles_alone() -> Result<()> {
    let (it, _, _) = installs()?;
    let serve = RoleForm {
        install: &it,
        alias: true,
        prefix: Some("serve"),
    };
    let access = RoleForm {
        install: &it,
        alias: true,
        prefix: Some("access"),
    };
    roundtrip(&serve, &access, false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn standalone_aliases_forward_with_short_options() -> Result<()> {
    let (_, its, ita) = installs()?;
    let serve = RoleForm {
        install: &its,
        alias: true,
        prefix: None,
    };
    let access = RoleForm {
        install: &ita,
        alias: true,
        prefix: None,
    };
    roundtrip(&serve, &access, true).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn standalone_full_names_forward_with_long_options() -> Result<()> {
    let (_, its, ita) = installs()?;
    let serve = RoleForm {
        install: &its,
        alias: false,
        prefix: None,
    };
    let access = RoleForm {
        install: &ita,
        alias: false,
        prefix: None,
    };
    roundtrip(&serve, &access, false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn all_in_one_and_standalone_roles_interoperate_with_short_options() -> Result<()> {
    let (it, its, ita) = installs()?;
    roundtrip(
        &RoleForm {
            install: &it,
            alias: false,
            prefix: Some("serve"),
        },
        &RoleForm {
            install: &ita,
            alias: true,
            prefix: None,
        },
        true,
    )
    .await?;
    roundtrip(
        &RoleForm {
            install: &its,
            alias: true,
            prefix: None,
        },
        &RoleForm {
            install: &it,
            alias: false,
            prefix: Some("access"),
        },
        true,
    )
    .await
}

fn write_key(dir: &Path, seed: u8) -> Result<(PathBuf, String)> {
    let key = iroh::SecretKey::from_bytes(&[seed; 32]);
    let path = dir.join("identity.key");
    std::fs::write(
        &path,
        format!("{}\n", iroh_tunnel::config::encode_secret_key(&key)),
    )?;
    Ok((path, key.public().to_string()))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn supplied_key_keeps_public_identity_across_restarts_for_every_role_form() -> Result<()> {
    let (it, its, ita) = installs()?;
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let relay = relay.to_string();
    let peer = iroh::SecretKey::from_bytes(&[9; 32]).public().to_string();
    let forms = [
        (
            "it serve",
            RoleForm {
                install: &it,
                alias: true,
                prefix: Some("serve"),
            },
            1u8,
        ),
        (
            "it access",
            RoleForm {
                install: &it,
                alias: true,
                prefix: Some("access"),
            },
            2,
        ),
        (
            "its",
            RoleForm {
                install: &its,
                alias: true,
                prefix: None,
            },
            3,
        ),
        (
            "ita",
            RoleForm {
                install: &ita,
                alias: true,
                prefix: None,
            },
            4,
        ),
    ];
    for (label, form, seed) in &forms {
        let keys = tempfile::tempdir()?;
        let (key_path, expected) = write_key(keys.path(), *seed)?;
        let key_bytes = std::fs::read(&key_path)?;
        let is_serve = matches!(*label, "it serve" | "its");
        let mut ids = Vec::new();
        for short in [true, false] {
            let state = tempfile::tempdir()?;
            let id = if is_serve {
                start_serve(
                    form,
                    state.path(),
                    short,
                    "127.0.0.1:9",
                    &relay,
                    Some(&key_path),
                )
                .await?
                .1
            } else {
                start_access(form, state.path(), short, &peer, &relay, Some(&key_path))
                    .await?
                    .1
            };
            ids.push(id);
        }
        eprintln!("{label} restart public identities (short, long): {ids:?}");
        assert_eq!(
            ids,
            [expected.clone(), expected],
            "{label} identity changed"
        );
        assert_eq!(std::fs::read(&key_path)?, key_bytes, "{label} rewrote key");
        let written = non_ownership_files(&form.install.home())?;
        assert!(written.is_empty(), "{label} wrote config: {written:?}");
    }
    Ok(())
}

/// Files under `dir` other than role-ownership runtime state (#87: lock and
/// owner records in the default state directory), which is neither config
/// nor keys.
fn non_ownership_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            found.extend(non_ownership_files(&path)?);
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let ownership = path
            .parent()
            .is_some_and(|p| p.ends_with(".local/state/iroh-tunnel"))
            && (name.ends_with(".lock") || name.contains(".owner.json"));
        if !ownership {
            found.push(path);
        }
    }
    Ok(found)
}

fn help(form: &RoleForm<'_>) -> Result<String> {
    let output = form.output(&["--help"])?;
    assert!(output.status.success(), "{output:?}");
    Ok(String::from_utf8(output.stdout)?)
}

#[test]
fn help_lists_role_options_aliases_and_defaults() -> Result<()> {
    let (it, its, ita) = installs()?;
    let top = help(&RoleForm {
        install: &it,
        alias: true,
        prefix: None,
    })?;
    for expected in ["serve", "access", "it", "its", "ita"] {
        assert!(top.contains(expected), "it --help lacks {expected}: {top}");
    }
    let serve_options = [
        "-n, --name",
        "-t, --target",
        "-p, --protocol",
        "-k, --key-file",
    ];
    let access_options = [
        "-n, --name",
        "-l, --listen",
        "-N, --node-id",
        "-p, --protocol",
        "-k, --key-file",
    ];
    let cases = [
        (
            "it serve",
            RoleForm {
                install: &it,
                alias: true,
                prefix: Some("serve"),
            },
            &serve_options[..],
            "its",
        ),
        (
            "its",
            RoleForm {
                install: &its,
                alias: true,
                prefix: None,
            },
            &serve_options[..],
            "its",
        ),
        (
            "it access",
            RoleForm {
                install: &it,
                alias: true,
                prefix: Some("access"),
            },
            &access_options[..],
            "ita",
        ),
        (
            "ita",
            RoleForm {
                install: &ita,
                alias: true,
                prefix: None,
            },
            &access_options[..],
            "ita",
        ),
    ];
    for (label, form, options, alias) in &cases {
        let text = help(form)?;
        for option in options.iter().chain(&["--relay-url", "[default: tcp]"]) {
            assert!(text.contains(option), "{label} help lacks {option}: {text}");
        }
        for command in ["run", "config", "status", "service"] {
            assert!(
                text.contains(command),
                "{label} help lacks {command}: {text}"
            );
        }
        assert!(
            text.contains(alias),
            "{label} help lacks alias {alias}: {text}"
        );
        eprintln!("{label} --help:\n{text}");
    }
    for form in [
        RoleForm {
            install: &its,
            alias: true,
            prefix: None,
        },
        RoleForm {
            install: &ita,
            alias: true,
            prefix: None,
        },
    ] {
        let version = form.output(&["-V"])?;
        assert!(version.status.success(), "{version:?}");
    }
    Ok(())
}

fn assert_rejected(form: &RoleForm<'_>, args: &[&str], diagnostic: &str) -> Result<String> {
    let output = form.output(args)?;
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(!output.status.success(), "{args:?} accepted: {stderr}");
    assert!(stderr.contains(diagnostic), "{args:?}: {stderr}");
    assert_eq!(std::fs::read_dir(form.install.home())?.count(), 0);
    eprintln!("rejected {args:?} ({:?}): {stderr}", output.status.code());
    Ok(stderr)
}

#[test]
fn role_specific_options_are_rejected_by_the_other_role() -> Result<()> {
    let (it, its, ita) = installs()?;
    let peer = iroh::SecretKey::from_bytes(&[9; 32]).public().to_string();
    let serve_forms = [
        RoleForm {
            install: &it,
            alias: true,
            prefix: Some("serve"),
        },
        RoleForm {
            install: &its,
            alias: true,
            prefix: None,
        },
    ];
    let access_forms = [
        RoleForm {
            install: &it,
            alias: true,
            prefix: Some("access"),
        },
        RoleForm {
            install: &ita,
            alias: true,
            prefix: None,
        },
    ];
    for form in &serve_forms {
        for (flag, value) in [
            ("-l", "127.0.0.1:1"),
            ("--listen", "127.0.0.1:1"),
            ("-N", &peer),
            ("--node-id", &peer),
        ] {
            assert_rejected(
                form,
                &["-n", "echo", "-t", "127.0.0.1:1", flag, value],
                flag,
            )?;
        }
    }
    for form in &access_forms {
        for flag in ["-t", "--target"] {
            assert_rejected(
                form,
                &[
                    "-n",
                    "echo",
                    "-l",
                    "127.0.0.1:1",
                    "-N",
                    &peer,
                    flag,
                    "127.0.0.1:1",
                ],
                flag,
            )?;
        }
    }
    // A standalone binary has no role subcommands.
    assert_rejected(
        &RoleForm {
            install: &its,
            alias: true,
            prefix: None,
        },
        &["access"],
        "access",
    )?;
    assert_rejected(
        &RoleForm {
            install: &ita,
            alias: true,
            prefix: None,
        },
        &["serve"],
        "serve",
    )?;
    Ok(())
}

#[test]
fn unusable_key_files_fail_without_revealing_contents_or_writing_config() -> Result<()> {
    let (it, its, ita) = installs()?;
    let peer = iroh::SecretKey::from_bytes(&[9; 32]).public().to_string();
    let keys = tempfile::tempdir()?;
    let missing = keys.path().join("missing.key");
    let invalid = keys.path().join("invalid.key");
    let secret_looking = "c2VjcmV0LWJ1dC10b28tc2hvcnQ=";
    std::fs::write(&invalid, secret_looking)?;
    let cases: [(RoleForm<'_>, Vec<&str>); 4] = [
        (
            RoleForm {
                install: &it,
                alias: true,
                prefix: Some("serve"),
            },
            vec!["-n", "echo", "-t", "127.0.0.1:1"],
        ),
        (
            RoleForm {
                install: &its,
                alias: true,
                prefix: None,
            },
            vec!["-n", "echo", "-t", "127.0.0.1:1"],
        ),
        (
            RoleForm {
                install: &it,
                alias: true,
                prefix: Some("access"),
            },
            vec!["-n", "echo", "-l", "127.0.0.1:1", "-N", &peer],
        ),
        (
            RoleForm {
                install: &ita,
                alias: true,
                prefix: None,
            },
            vec!["-n", "echo", "-l", "127.0.0.1:1", "-N", &peer],
        ),
    ];
    for (form, base) in &cases {
        for key in [&missing, &invalid] {
            let mut args = base.clone();
            args.extend(["-k", key.to_str().expect("utf-8 temp path")]);
            let stderr = assert_rejected(form, &args, "--key-file")?;
            assert!(
                !stderr.contains(secret_looking),
                "key contents leaked: {stderr}"
            );
        }
    }
    Ok(())
}

#[test]
fn standalone_service_install_is_refused_instead_of_writing_a_broken_unit() -> Result<()> {
    let (_, its, ita) = installs()?;
    for install in [&its, &ita] {
        let form = RoleForm {
            install,
            alias: true,
            prefix: None,
        };
        let output = form.output(&["service", "install"])?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(5), "{stderr}");
        assert!(stderr.contains("not yet supported"), "{stderr}");
        assert_eq!(std::fs::read_dir(install.home())?.count(), 0);
    }
    Ok(())
}
