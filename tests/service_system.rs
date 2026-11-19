//! System-tier service management on a BusyBox host, driven through the real
//! binary as real root, with a real unprivileged account for the user tier.
//!
//! Acceptance map (`elevation-derived-tiers`):
//! - Root installs a system service: `/etc/iroh-tunnel/<role>.toml` and
//!   `/run/iroh-tunnel` are pinned with the service marker and no pin; the
//!   invoking user's `HOME` / `SUDO_USER` are ignored.
//! - Missing system config: exit 2 naming the path and `config keygen`.
//! - User-owned config at install: exit 3 naming the path and the owner.
//! - Config loosened after install: the service refuses to run.
//! - Administrator edits config / Invalid edit / Missing key: applied after a
//!   restart, never served when invalid, key persisted; owner and mode kept.
//! - Pinned registration after upgrade (+ boot start of a legacy
//!   registration): config error naming the system path and the rerun.
//! - Boot start of a current registration: no binary check on the run path.
//! - Non-elevated user cannot read the system config.
//! - System install while a user service runs: both tiers run, with notices.
//! - Non-root BusyBox operation: permission error, nothing changed.
//! - SM-BIN-01..04, 06..09: system binary trust and managed copies.
//!
//! Needs real root, a real account and an init system the test may register
//! with, so it is opt-in: `IROH_TUNNEL_E2E_PRIVILEGED=1` in a disposable Linux
//! container without systemd (the BusyBox backend), run with
//! `--test-threads=1`. It adds the user `it91a`, writes `/etc/iroh-tunnel`,
//! `/etc/init.d/S96iroh-tunnel-*` and `/var/lib/iroh-tunnel`, and removes them
//! between tests. The `test-utils` feature disables relay TLS verification:
//! only an unreachable relay is used, never public relays or real secrets.
#![cfg(test)]
#![cfg(all(target_os = "linux", feature = "test-utils"))]

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_iroh-tunnel");
const SCRIPT: &str = "/etc/init.d/S96iroh-tunnel-serve";
const LOG: &str = "/var/log/iroh-tunnel-serve.log";
const PIDFILE: &str = "/var/run/iroh-tunnel-serve.pid";
const SYSTEM_CONFIG_DIR: &str = "/etc/iroh-tunnel";
const SYSTEM_CONFIG: &str = "/etc/iroh-tunnel/serve.toml";
const SYSTEM_STATE: &str = "/run/iroh-tunnel";
const SYSTEM_STATUS: &str = "/run/iroh-tunnel/serve-status.json";
const MANAGED_DIR: &str = "/var/lib/iroh-tunnel/bin/serve";
const MANAGED_COPY: &str = "/var/lib/iroh-tunnel/bin/serve/iroh-tunnel";
const SYSTEM_BIN: &str = "/usr/local/bin/iroh-tunnel";
const SYSTEM_PATH: &str = "/usr/local/bin:/usr/bin:/bin";
const NOTE: &str = "Registered a root-owned copy of";
const USER_NOTICE: &str = "note: serve is also running in a user session";
const SYSTEM_NOTICE: &str = "note: serve is also running as a system service";
/// Test-build seams a stray environment must not leak into the binary.
const TEST_SEAMS: [&str; 5] = [
    "IROH_TUNNEL_TEST_TIER",
    "IROH_TUNNEL_TEST_CONFIG_DIR",
    "IROH_TUNNEL_TEST_TRUSTED_OWNER",
    "IROH_TUNNEL_TEST_MANAGED_BIN_DIR",
    "IROH_TUNNEL_TEST_INTERACTIVE",
];
/// A `test-utils` build keeps its machine lock directory beside the user
/// tier unless told otherwise; every process here (root, the registered
/// service it starts, and the unprivileged user) is pointed at the
/// production location so the tiers see each other as in a release build.
const MACHINE_LOCK_DIR: (&str, &str) = ("IROH_TUNNEL_MACHINE_LOCK_DIR", SYSTEM_STATE);

fn enabled() -> bool {
    // SAFETY: geteuid(2) has no preconditions and cannot fail.
    let root = unsafe { libc::geteuid() } == 0;
    let on = std::env::var_os("IROH_TUNNEL_E2E_PRIVILEGED").is_some();
    if !(on && root) {
        eprintln!("skipped: set IROH_TUNNEL_E2E_PRIVILEGED=1 as root in a disposable container");
    }
    on && root
}

fn sh(args: &[&str]) -> Output {
    Command::new(args[0]).args(&args[1..]).output().unwrap()
}

fn ensure_user(name: &str) -> (u32, PathBuf) {
    if sh(&["id", "-u", name]).status.code() != Some(0) {
        assert!(sh(&["useradd", "-m", "-s", "/bin/sh", name])
            .status
            .success());
    }
    let uid = String::from_utf8(sh(&["id", "-u", name]).stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    (uid, PathBuf::from(format!("/home/{name}")))
}

fn mode(path: &Path, mode: u32) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

fn own(path: &Path, uid: u32) {
    std::os::unix::fs::chown(path, Some(uid), Some(uid)).unwrap();
}

fn owner_and_mode(path: &Path) -> (u32, u32) {
    let meta = std::fs::metadata(path).unwrap();
    (meta.uid(), meta.mode() & 0o7777)
}

/// A serve config with one service `service` and `secret_key`.
fn config_text(service: &str, secret_key: &str) -> String {
    format!(
        "[node]\nsecret_key = {secret_key:?}\nrelay_urls = [\"https://relay.invalid\"]\n\n\
         [[services]]\nname = \"{service}\"\nprotocol = \"tcp\"\nport = 9\n"
    )
}

fn fresh_key() -> String {
    iroh_tunnel_host::config::encode_secret_key(&iroh::SecretKey::generate())
}

/// Write `/etc/iroh-tunnel/serve.toml` as root: directory 0755, file 0600.
fn write_system_config(service: &str, secret_key: &str) {
    std::fs::create_dir_all(SYSTEM_CONFIG_DIR).unwrap();
    mode(Path::new(SYSTEM_CONFIG_DIR), 0o755);
    std::fs::write(SYSTEM_CONFIG, config_text(service, secret_key)).unwrap();
    own(Path::new(SYSTEM_CONFIG), 0);
    mode(Path::new(SYSTEM_CONFIG), 0o600);
}

/// An unprivileged account with its own user-tier config, state and `bin`.
struct Home {
    uid: u32,
    home: PathBuf,
}

impl Home {
    fn new(name: &'static str, service: &str) -> Self {
        let (uid, home) = ensure_user(name);
        let h = Self { uid, home };
        std::fs::create_dir_all(h.config().parent().unwrap()).unwrap();
        std::fs::create_dir_all(h.state()).unwrap();
        std::fs::write(h.config(), config_text(service, &fresh_key())).unwrap();
        own(&h.config(), uid);
        mode(&h.config(), 0o600);
        for dir in [
            h.home.clone(),
            h.home.join(".config"),
            h.home.join(".config/iroh-tunnel"),
            h.home.join(".local"),
            h.home.join(".local/state"),
            h.state(),
        ] {
            own(&dir, uid);
            mode(&dir, 0o755);
        }
        h
    }
    fn config(&self) -> PathBuf {
        self.home.join(".config/iroh-tunnel/serve.toml")
    }
    fn state(&self) -> PathBuf {
        self.home.join(".local/state/iroh-tunnel")
    }
    fn status_file(&self) -> PathBuf {
        self.state().join("serve-status.json")
    }
    /// `args` run as this account (not root), with its own `HOME`.
    fn command(&self, program: &str, args: &[&str]) -> Command {
        let uid = self.uid.to_string();
        let mut command = Command::new("setpriv");
        command
            .args(["--reuid", &uid, "--regid", &uid, "--clear-groups", "--"])
            .arg(program)
            .args(args)
            .env("HOME", &self.home)
            .env(MACHINE_LOCK_DIR.0, MACHINE_LOCK_DIR.1)
            .env_remove("IROH_TUNNEL_STATE_DIR")
            .stdin(Stdio::null());
        for seam in TEST_SEAMS {
            command.env_remove(seam);
        }
        command
    }
}

/// `args` run as root with root's `HOME`, no `SUDO_USER` and no overrides.
fn root_command(args: &[&str]) -> Command {
    let mut command = Command::new(BIN);
    command
        .args(args)
        .env("HOME", "/root")
        .env(MACHINE_LOCK_DIR.0, MACHINE_LOCK_DIR.1)
        .env_remove("SUDO_USER")
        .env_remove("IROH_TUNNEL_STATE_DIR")
        .stdin(Stdio::null());
    for seam in TEST_SEAMS {
        command.env_remove(seam);
    }
    command
}

fn run(args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut command = root_command(args);
    command.envs(envs.iter().copied());
    command.output().unwrap()
}

fn err(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn script() -> String {
    std::fs::read_to_string(SCRIPT).unwrap_or_default()
}

fn tail_log() -> String {
    std::fs::read_to_string(LOG).unwrap_or_default()
}

fn wait_for(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if ready() {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    panic!("timeout waiting for {what}; log: {}", tail_log());
}

fn install() -> Output {
    run(&["serve", "service", "install"], &[])
}

fn service_verb(verb: &str, path: &str) -> Output {
    run(&["serve", "service", verb], &[("PATH", path)])
}

fn restart() -> Output {
    run(&["serve", "service", "restart"], &[])
}

fn system_status() -> Option<serde_json::Value> {
    serde_json::from_str(&std::fs::read_to_string(SYSTEM_STATUS).ok()?).ok()
}

fn status_names(status: &serde_json::Value) -> Vec<String> {
    status["services"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s["name"].as_str().map(str::to_owned))
        .collect()
}

/// The binary the init script runs: `DAEMON=` as a plain word or
/// `'…'` segments joined by `\'`, or the legacy `DAEMON="…"` form.
fn daemon() -> String {
    script()
        .lines()
        .find_map(|l| l.strip_prefix("DAEMON="))
        .map(shell_word)
        .unwrap_or_default()
}

/// Decode one shell word built from plain characters, `'…'` segments and
/// `\'`, or a legacy double-quoted word (taken verbatim between the quotes).
fn shell_word(word: &str) -> String {
    if let Some(inner) = word.strip_prefix('"').and_then(|w| w.strip_suffix('"')) {
        return inner.to_owned();
    }
    let mut out = String::new();
    let mut chars = word.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => out.extend(chars.by_ref().take_while(|&c| c != '\'')),
            '\\' => out.extend(chars.next()),
            c => out.push(c),
        }
    }
    out
}

/// The PID on the first line of the serve PID file, or `""`.
fn pidfile_pid() -> String {
    pid_in(PIDFILE).unwrap_or_default()
}

fn pid_in(pidfile: &str) -> Option<String> {
    let text = std::fs::read_to_string(pidfile).ok()?;
    Some(text.lines().next().unwrap_or_default().trim().to_owned())
}

fn exited(pid: &str) -> bool {
    // Without an init to reap it, an exited child stays a zombie (`Z`).
    std::fs::read_to_string(format!("/proc/{pid}/stat")).map_or(true, |stat| {
        stat.rsplit(") ").next().is_some_and(|r| r.starts_with('Z'))
    })
}

fn alive(pid: &str) -> bool {
    !exited(pid)
}

/// The start time (field 22 of `/proc/<pid>/stat`) of `pid`, if it exists.
fn start_time(pid: &str) -> Option<String> {
    if pid.is_empty() {
        return None;
    }
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit(") ")
        .next()?
        .split_whitespace()
        .nth(19)
        .map(str::to_owned)
}

/// Wait until the stopped service's process is gone and its pidfile removed.
fn wait_released(pid: &str) {
    wait_for("the service process to exit", || exited(pid));
    wait_for("the pidfile to clear", || !Path::new(PIDFILE).exists());
}

/// `service stop`, then wait for the release of the pidfile and role lock.
fn stop_and_release(path: &str) {
    let pid = pidfile_pid();
    let out = service_verb("stop", path);
    assert!(out.status.success(), "stop: {}", err(&out));
    wait_released(&pid);
}

/// Drop a snapshot left by an earlier process.
fn clear_snapshot() {
    let _ = std::fs::remove_file(SYSTEM_STATUS);
}

/// Wait until the live process in the pidfile has written its own snapshot.
fn wait_running() {
    wait_for("the live service to report its own status", || {
        let pid = pidfile_pid();
        !pid.is_empty()
            && alive(&pid)
            && system_status()
                .and_then(|s| s["pid"].as_u64())
                .map(|p| p.to_string())
                == Some(pid)
    });
}

/// Restart and require a refusal logged as `needle` that never runs.
fn restart_expecting_refusal(needle: &str) -> String {
    clear_snapshot();
    let _ = std::fs::remove_file(LOG);
    let out = restart();
    assert!(out.status.success(), "restart: {}", err(&out));
    wait_for("the refusal to be logged", || tail_log().contains(needle));
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        system_status().is_none(),
        "the refused service must not run"
    );
    tail_log()
}

// --- Sandbox ---------------------------------------------------------------

const ACCESS_SCRIPT: &str = "/etc/init.d/S96iroh-tunnel-access";
const ACCESS_PIDFILE: &str = "/var/run/iroh-tunnel-access.pid";
const ACCESS_DIR: &str = "/var/lib/iroh-tunnel/bin/access";
const ACCESS_COPY: &str = "/var/lib/iroh-tunnel/bin/access/iroh-tunnel";
const ACCESS_CONFIG: &str = "/etc/iroh-tunnel/access.toml";
const BREW_PREFIX: &str = "/home/linuxbrew/.linuxbrew";

/// Start from nothing and put everything these tests touch back on drop.
struct Sandbox;

impl Sandbox {
    fn new() -> Self {
        reset();
        Self
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        reset();
    }
}

/// Stop and remove the registration of `script`, killing a process that
/// ignored `stop` so it cannot outlive its binaries.
fn reset_role(script: &str, pidfile: &str, role: &str) {
    let pid = pid_in(pidfile).unwrap_or_default();
    // Identify the process by pid and start time, to reduce the risk of
    // killing or waiting on a reused pid.
    let started = start_time(&pid);
    let same_alive = || started.is_some() && start_time(&pid) == started && alive(&pid);
    if Path::new(script).exists() {
        let _ = Command::new("sh").args([script, "stop"]).output();
    }
    if same_alive() {
        if let Ok(n) = pid.parse::<i32>() {
            // SAFETY: kill(2) takes plain integers and has no memory effects.
            unsafe { libc::kill(n, libc::SIGKILL) };
        }
    }
    wait_for("the previous service process to exit", || !same_alive());
    if Path::new(script).exists() {
        let _ = root_command(&[role, "service", "uninstall"]).output();
        let _ = std::fs::remove_file(script);
    }
    let _ = std::fs::remove_file(pidfile);
}

/// Remove every registration, process, binary copy, config and snapshot
/// these tests create, whatever state a previous test left.
fn reset() {
    reset_role(SCRIPT, PIDFILE, "serve");
    reset_role(ACCESS_SCRIPT, ACCESS_PIDFILE, "access");
    let _ = std::fs::remove_file(LOG);
    let _ = std::fs::remove_dir_all(MANAGED_DIR);
    let _ = std::fs::remove_dir_all(ACCESS_DIR);
    let _ = std::fs::remove_file(SYSTEM_BIN);
    let _ = std::fs::remove_dir_all(SYSTEM_CONFIG_DIR);
    let _ = std::fs::remove_file(SYSTEM_STATUS);
    let _ = std::fs::remove_dir_all("/home/linuxbrew");
    // Everything the fixture account gets from `Home::new` or a user-tier
    // run (config, state with snapshots and locks, run log), so a re-run
    // starts from an empty user tier.
    for path in [
        "/home/it91a/bin",
        "/home/it91a/.cargo",
        "/home/it91a/.config/iroh-tunnel",
        "/home/it91a/.local/state/iroh-tunnel",
    ] {
        let _ = std::fs::remove_dir_all(path);
    }
    let _ = std::fs::remove_file("/home/it91a/user-run.log");
}

/// A user-tier process that is killed when the test ends.
struct UserInstance(Child);

impl Drop for UserInstance {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

// --- System tier paths and trust -------------------------------------------

#[test]
fn system_install_uses_etc_and_run() {
    if !enabled() {
        return;
    }
    let _sandbox = Sandbox::new();
    let alice = Home::new("it91a", "user-side");
    write_system_config("alpha", &fresh_key());

    // A `sudo -E` style environment: the invoking user's HOME and SUDO_USER
    // must not influence the system tier.
    let home = alice.home.to_str().unwrap().to_owned();
    let out = run(
        &["serve", "service", "install"],
        &[("HOME", &home), ("SUDO_USER", "it91a")],
    );
    assert!(out.status.success(), "{}", err(&out));
    let registered = script();
    assert!(
        registered.contains(&format!("CONFIG={SYSTEM_CONFIG}\n")),
        "{registered}"
    );
    assert!(
        registered.contains(&format!("export IROH_TUNNEL_STATE_DIR={SYSTEM_STATE}\n")),
        "{registered}"
    );
    assert!(
        registered.contains("export IROH_TUNNEL_SERVICE=1\n"),
        "{registered}"
    );
    assert!(!registered.contains("TRUSTED_UID"), "{registered}");
    assert!(!registered.contains("/home/"), "{registered}");
    assert!(!registered.contains("/root"), "{registered}");
    // The install creates the presence file a later user instance holds.
    assert_eq!(
        owner_and_mode(&Path::new(SYSTEM_STATE).join("serve.user.lock")),
        (0, 0o644)
    );
    wait_running();
    assert!(status_names(&system_status().unwrap()).contains(&"alpha".to_owned()));
    // Nothing was written under the invoking user's home.
    assert!(!alice.status_file().exists());
}

#[test]
fn missing_system_config_names_keygen() {
    if !enabled() {
        return;
    }
    let _sandbox = Sandbox::new();
    let out = install();
    let text = err(&out);
    assert_eq!(out.status.code(), Some(2), "{text}");
    assert!(
        text.contains(SYSTEM_CONFIG) && text.contains("sudo iroh-tunnel serve config keygen"),
        "{text}"
    );
    assert!(!Path::new(SCRIPT).exists(), "nothing may be registered");
}

#[test]
fn user_owned_config_refused_at_install() {
    if !enabled() {
        return;
    }
    let _sandbox = Sandbox::new();
    let alice = Home::new("it91a", "alpha");
    let config = alice.config();
    let before = (std::fs::read(&config).unwrap(), owner_and_mode(&config));

    let out = run(
        &[
            "serve",
            "service",
            "install",
            "--config",
            config.to_str().unwrap(),
        ],
        &[],
    );
    let text = err(&out);
    assert_eq!(out.status.code(), Some(3), "{text}");
    assert!(text.contains("permission error"), "{text}");
    assert!(
        text.contains(alice.home.to_str().unwrap()) && text.contains(&format!("uid {}", alice.uid)),
        "{text}"
    );
    assert!(!Path::new(SCRIPT).exists(), "nothing may be registered");
    assert_eq!(
        (std::fs::read(&config).unwrap(), owner_and_mode(&config)),
        before
    );
}

#[test]
fn loosened_config_refused_at_service_start() {
    if !enabled() {
        return;
    }
    let _sandbox = Sandbox::new();
    write_system_config("alpha", &fresh_key());
    let out = install();
    assert!(out.status.success(), "{}", err(&out));
    wait_running();

    mode(Path::new(SYSTEM_CONFIG), 0o666);
    let log = restart_expecting_refusal("permission error");
    assert!(log.contains(SYSTEM_CONFIG) && log.contains("0666"), "{log}");
    // The service never repairs access.
    assert_eq!(owner_and_mode(Path::new(SYSTEM_CONFIG)), (0, 0o666));

    // Once the administrator fixes it, the service runs again.
    mode(Path::new(SYSTEM_CONFIG), 0o600);
    clear_snapshot();
    let out = restart();
    assert!(out.status.success(), "restart: {}", err(&out));
    wait_running();
}

#[test]
fn admin_edit_applies_after_restart_keeping_root_0600() {
    if !enabled() {
        return;
    }
    let _sandbox = Sandbox::new();
    write_system_config("alpha", &fresh_key());
    let out = install();
    assert!(out.status.success(), "{}", err(&out));
    wait_running();

    // The administrator edits the default (system-tier) config.
    let out = run(
        &[
            "serve", "config", "set", "-n", "bravo", "-p", "tcp", "-t", "9",
        ],
        &[],
    );
    assert!(out.status.success(), "config set: {}", err(&out));
    clear_snapshot();
    let out = restart();
    assert!(out.status.success(), "restart: {}", err(&out));
    wait_running();
    assert!(
        status_names(&system_status().unwrap()).contains(&"bravo".to_owned()),
        "{:?}",
        system_status()
    );
    assert_eq!(owner_and_mode(Path::new(SYSTEM_CONFIG)), (0, 0o600));
}

#[test]
fn invalid_edit_never_serves_and_keeps_owner_mode() {
    if !enabled() {
        return;
    }
    let _sandbox = Sandbox::new();
    write_system_config("alpha", &fresh_key());
    let out = install();
    assert!(out.status.success(), "{}", err(&out));
    wait_running();

    // Rewriting in place keeps the inode, owner and mode.
    std::fs::write(SYSTEM_CONFIG, "[node]\nnot valid toml = =\n").unwrap();
    restart_expecting_refusal("failed to parse");
    assert_eq!(owner_and_mode(Path::new(SYSTEM_CONFIG)), (0, 0o600));
}

#[test]
fn system_service_persists_generated_key_root_0600() {
    if !enabled() {
        return;
    }
    let _sandbox = Sandbox::new();
    write_system_config("alpha", "");
    let out = install();
    assert!(out.status.success(), "{}", err(&out));
    wait_running();

    let doc: toml::Value =
        toml::from_str(&std::fs::read_to_string(SYSTEM_CONFIG).unwrap()).unwrap();
    let key = doc["node"]["secret_key"].as_str().unwrap().to_owned();
    assert!(!key.is_empty(), "the generated key must be persisted");
    assert_eq!(owner_and_mode(Path::new(SYSTEM_CONFIG)), (0, 0o600));
    let first = system_status().unwrap()["node_id"].clone();

    // The persisted key is the identity after a restart.
    clear_snapshot();
    let out = restart();
    assert!(out.status.success(), "restart: {}", err(&out));
    wait_running();
    assert_eq!(system_status().unwrap()["node_id"], first);
}

#[test]
fn non_root_cannot_read_system_config() {
    if !enabled() {
        return;
    }
    let _sandbox = Sandbox::new();
    let alice = Home::new("it91a", "alpha");
    let out = run(&["serve", "config", "keygen"], &[]);
    assert!(out.status.success(), "keygen: {}", err(&out));
    assert_eq!(owner_and_mode(Path::new(SYSTEM_CONFIG_DIR)), (0, 0o755));
    assert_eq!(owner_and_mode(Path::new(SYSTEM_CONFIG)), (0, 0o600));

    let out = alice.command("cat", &[SYSTEM_CONFIG]).output().unwrap();
    assert!(!out.status.success(), "a user read the system config");
    assert!(err(&out).contains("Permission denied"), "{}", err(&out));
    assert!(out.stdout.is_empty(), "no secret bytes may leak");
}

#[test]
fn user_and_system_tiers_coexist_with_notices() {
    if !enabled() {
        return;
    }
    let _sandbox = Sandbox::new();
    let alice = Home::new("it91a", "user-side");
    write_system_config("alpha", &fresh_key());
    let out = install();
    assert!(out.status.success(), "{}", err(&out));
    wait_running();
    let system_pid = pidfile_pid();

    // Alice runs her own user-tier instance with her own key and files.
    let _ = std::fs::remove_file(alice.status_file());
    let user_log = alice.home.join("user-run.log");
    let mut user = alice.command(
        BIN,
        &["serve", "run", "--config", alice.config().to_str().unwrap()],
    );
    user.stdout(Stdio::null())
        .stderr(std::fs::File::create(&user_log).unwrap());
    let mut user = UserInstance(user.spawn().unwrap());
    wait_for("the user instance to report status", || {
        alice.status_file().exists()
    });
    let user_err = std::fs::read_to_string(&user_log).unwrap();
    assert!(user_err.contains(SYSTEM_NOTICE), "{user_err}");
    assert!(user.0.try_wait().unwrap().is_none(), "user instance exited");
    assert!(alive(&system_pid), "the system service must keep running");

    // A system re-install while the user instance runs proceeds with a note.
    clear_snapshot();
    let out = install();
    assert!(out.status.success(), "{}", err(&out));
    assert!(err(&out).contains(USER_NOTICE), "{}", err(&out));
    wait_running();
    assert!(
        user.0.try_wait().unwrap().is_none(),
        "user instance stopped"
    );

    // System status tells the administrator about the user session.
    let out = run(&["serve", "status"], &[]);
    assert!(err(&out).contains(USER_NOTICE), "{}", err(&out));
    assert!(
        !err(&out).contains(alice.home.to_str().unwrap()),
        "the notice names no account: {}",
        err(&out)
    );
}

#[test]
fn busybox_non_root_mutation_refused() {
    if !enabled() {
        return;
    }
    let _sandbox = Sandbox::new();
    let alice = Home::new("it91a", "alpha");
    write_system_config("alpha", &fresh_key());
    let out = install();
    assert!(out.status.success(), "{}", err(&out));
    wait_running();
    let registration = script();
    let pid = pidfile_pid();
    let started = start_time(&pid);

    for verb in ["install", "uninstall", "start", "stop", "restart"] {
        let out = alice
            .command(BIN, &["serve", "service", verb])
            .output()
            .unwrap();
        let text = err(&out);
        assert_eq!(out.status.code(), Some(3), "{verb}: {text}");
        assert!(
            text.contains("no per-user services") && text.contains("sudo"),
            "{verb}: {text}"
        );
        assert_eq!(script(), registration, "{verb} changed the registration");
        assert_eq!(start_time(&pid), started, "{verb} touched the service");
    }
    assert!(!alice.home.join(".config/systemd").exists());
}

#[test]
fn legacy_pinned_script_is_refused_with_guidance() {
    if !enabled() {
        return;
    }
    let _sandbox = Sandbox::new();
    let alice = Home::new("it91a", "alpha");
    place_user_binary(&alice);
    write_system_config("alpha", &fresh_key());
    let out = install();
    assert!(out.status.success(), "{}", err(&out));
    wait_running();
    stop_and_release(SYSTEM_PATH);
    let alice_before = (
        std::fs::read(alice.config()).unwrap(),
        owner_and_mode(&alice.config()),
    );

    // A registration written by an earlier version: a user-writable binary,
    // alice's files and the trust pin instead of the service marker.
    let user_binary = user_bin(&alice);
    rewrite_script(|line| {
        if line.starts_with("DAEMON=") {
            format!("DAEMON={}", user_binary.display())
        } else if line.starts_with("CONFIG=") {
            format!("CONFIG={}", alice.config().display())
        } else if line.starts_with("export IROH_TUNNEL_STATE_DIR=") {
            format!("export IROH_TUNNEL_STATE_DIR={}", alice.state().display())
        } else if line == "export IROH_TUNNEL_SERVICE=1" {
            format!("export IROH_TUNNEL_TRUSTED_UID={}", alice.uid)
        } else {
            line.to_owned()
        }
    });
    assert_eq!(daemon(), user_binary.to_str().unwrap(), "{}", script());

    // The init system starts it at boot: no binary check, a config error.
    let _ = std::fs::remove_file(LOG);
    let status = Command::new("sh")
        .args([SCRIPT, "start"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    wait_for("the legacy refusal", || {
        tail_log().contains("earlier version")
    });
    let log = tail_log();
    assert!(
        log.contains("config error")
            && log.contains(SYSTEM_CONFIG)
            && log.contains("sudo iroh-tunnel serve service install"),
        "{log}"
    );
    let pid = pidfile_pid();
    wait_for("the refused process to exit", || exited(&pid));
    assert!(
        !alice.status_file().exists(),
        "the legacy service must not serve"
    );

    // The same start observed directly exits with the config error code.
    let out = root_command(&["serve", "run", "--config", alice.config().to_str().unwrap()])
        .env("IROH_TUNNEL_STATE_DIR", alice.state())
        .env("IROH_TUNNEL_TRUSTED_UID", alice.uid.to_string())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{}", err(&out));
    assert_eq!(
        (
            std::fs::read(alice.config()).unwrap(),
            owner_and_mode(&alice.config())
        ),
        alice_before,
        "alice's file must be unchanged"
    );
}

/// Rewrite the installed init script line by line.
fn rewrite_script(mut edit: impl FnMut(&str) -> String) {
    let text: String = script().lines().map(|l| edit(l) + "\n").collect();
    std::fs::write(SCRIPT, text).unwrap();
}

#[test]
fn boot_start_of_current_registration_is_not_binary_checked() {
    if !enabled() {
        return;
    }
    let _sandbox = Sandbox::new();
    let alice = Home::new("it91a", "alpha");
    place_user_binary(&alice);
    write_system_config("alpha", &fresh_key());
    let out = install();
    assert!(out.status.success(), "{}", err(&out));
    wait_running();
    stop_and_release(SYSTEM_PATH);

    // A current registration whose binary would fail the install check.
    let user_binary = user_bin(&alice);
    rewrite_script(|line| {
        if line.starts_with("DAEMON=") {
            format!("DAEMON={}", user_binary.display())
        } else {
            line.to_owned()
        }
    });
    assert_eq!(daemon(), user_binary.to_str().unwrap());

    clear_snapshot();
    let status = Command::new("sh")
        .args([SCRIPT, "start"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    wait_running();
}

// --- System binary trust (SM-BIN-*) ----------------------------------------

/// `PATH` that resolves `iroh-tunnel` to the user's own bin directory.
fn user_path(user: &Home) -> String {
    format!("{}:/usr/bin:/bin", user_bin_dir(user).display())
}

fn user_bin_dir(user: &Home) -> PathBuf {
    user.home.join("bin")
}

fn user_bin(user: &Home) -> PathBuf {
    user_bin_dir(user).join("iroh-tunnel")
}

/// Copy the binary under test to `dest` with the given owner and mode 0755.
fn place_binary(dest: &Path, uid: u32) {
    std::fs::copy(BIN, dest).unwrap();
    own(dest, uid);
    mode(dest, 0o755);
}

/// Put the binary under test in the user's own `bin` directory.
fn place_user_binary(user: &Home) {
    let dir = user_bin_dir(user);
    std::fs::create_dir_all(&dir).unwrap();
    own(&dir, user.uid);
    mode(&dir, 0o755);
    place_binary(&user_bin(user), user.uid);
}

/// Copy the binary under test to the root-owned `/usr/local/bin`.
fn place_system_binary() {
    place_binary(Path::new(SYSTEM_BIN), 0);
}

fn install_on_path(path: &str) -> Output {
    run(&["serve", "service", "install"], &[("PATH", path)])
}

#[test]
fn sm_bin_01_user_bin_dir_registers_managed_copy() {
    if !enabled() {
        return;
    }
    let _sandbox = Sandbox::new();
    let alice = Home::new("it91a", "alpha");
    write_system_config("alpha", &fresh_key());
    place_user_binary(&alice);
    let original = owner_and_mode(&user_bin(&alice));
    let original_bytes = std::fs::read(user_bin(&alice)).unwrap();

    clear_snapshot();
    let out = install_on_path(&user_path(&alice));
    assert!(out.status.success(), "{}", err(&out));

    assert_eq!(daemon(), MANAGED_COPY, "{}", script());
    assert_eq!(owner_and_mode(Path::new(MANAGED_COPY)), (0, 0o755));
    assert_eq!(
        std::fs::read(MANAGED_COPY).unwrap(),
        std::fs::read(BIN).unwrap()
    );
    assert_eq!(owner_and_mode(&user_bin(&alice)), original);
    assert_eq!(std::fs::read(user_bin(&alice)).unwrap(), original_bytes);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(NOTE)
            // The source is the running executable the install copied from.
            && stdout.contains(BIN)
            && stdout.contains(MANAGED_COPY)
            && stdout.contains("service install"),
        "{stdout}"
    );
    assert!(!stdout.contains("--system"), "{stdout}");
    wait_running();
}

#[test]
fn sm_bin_02_legacy_unsafe_daemon_refuses_start() {
    if !enabled() {
        return;
    }
    let _sandbox = Sandbox::new();
    let alice = Home::new("it91a", "alpha");
    write_system_config("alpha", &fresh_key());
    place_user_binary(&alice);
    let path = user_path(&alice);
    let user_binary = user_bin(&alice);
    let user_binary = user_binary.to_str().unwrap();

    clear_snapshot();
    let out = install_on_path(&path);
    assert!(out.status.success(), "{}", err(&out));
    wait_running();
    stop_and_release(&path);

    // A registration naming the user's own binary.
    let legacy = script().replace(&daemon(), user_binary);
    std::fs::write(SCRIPT, legacy).unwrap();
    assert_eq!(daemon(), user_binary);
    clear_snapshot();
    let out = service_verb("start", &path);
    let text = err(&out);
    assert_eq!(out.status.code(), Some(3), "{text}");
    // The refusal names the registered binary and the elevated reinstall.
    assert!(
        text.contains(user_binary) && text.contains("sudo iroh-tunnel serve service install"),
        "{text}"
    );
    assert!(!text.contains("--system"), "{text}");
    assert!(!Path::new(PIDFILE).exists(), "a refused start must not run");

    // Re-installing registers the managed copy, and start works again.
    clear_snapshot();
    let out = install_on_path(&path);
    assert!(out.status.success(), "{}", err(&out));
    assert_eq!(daemon(), MANAGED_COPY);
    wait_running();
    stop_and_release(&path);
    clear_snapshot();
    let out = service_verb("start", &path);
    assert!(out.status.success(), "start: {}", err(&out));
    wait_running();
}

#[test]
fn sm_bin_03_root_path_direct_and_uninstall_cleans() {
    if !enabled() {
        return;
    }
    let _sandbox = Sandbox::new();
    let alice = Home::new("it91a", "alpha");
    write_system_config("alpha", &fresh_key());
    place_user_binary(&alice);
    let path = user_path(&alice);

    clear_snapshot();
    let out = install_on_path(&path);
    assert!(out.status.success(), "{}", err(&out));
    assert!(Path::new(MANAGED_COPY).exists());
    wait_running();
    let pid = pidfile_pid();
    assert!(!pid.is_empty(), "no pid in {PIDFILE}");

    let out = service_verb("uninstall", &path);
    assert!(out.status.success(), "uninstall: {}", err(&out));
    assert!(std::fs::symlink_metadata(MANAGED_COPY).is_err());
    assert!(std::fs::symlink_metadata(MANAGED_DIR).is_err());
    wait_released(&pid);

    // A root-owned binary on PATH is registered directly.
    place_system_binary();
    let out = install_on_path(SYSTEM_PATH);
    assert!(out.status.success(), "{}", err(&out));
    assert_eq!(daemon(), SYSTEM_BIN, "{}", script());
    assert!(std::fs::symlink_metadata(MANAGED_COPY).is_err());
    assert!(!String::from_utf8_lossy(&out.stdout).contains(NOTE));
}

#[test]
fn sm_bin_04_role_dir_owned_by_user_refuses() {
    if !enabled() {
        return;
    }
    let _sandbox = Sandbox::new();
    let alice = Home::new("it91a", "alpha");
    write_system_config("alpha", &fresh_key());
    place_user_binary(&alice);
    std::fs::create_dir_all(MANAGED_DIR).unwrap();
    own(Path::new(MANAGED_DIR), alice.uid);
    mode(Path::new(MANAGED_DIR), 0o755);

    let out = install_on_path(&user_path(&alice));
    let text = err(&out);
    assert_eq!(out.status.code(), Some(3), "{text}");
    assert!(
        text.contains(MANAGED_DIR) && text.contains(&alice.uid.to_string()),
        "{text}"
    );
    assert!(
        !Path::new(SCRIPT).exists(),
        "no registration may be written"
    );
    assert_eq!(owner_and_mode(Path::new(MANAGED_DIR)), (alice.uid, 0o755));
}

// ---- Integrated coverage for unix-service-binary-trust (#208, verify W2) ----
//
// - SM-BIN-06: a root-owned binary below a user-owned directory is not trusted.
// - SM-BIN-07: a cargo bin directory and a Homebrew prefix register a copy.
// - SM-BIN-08: uninstalling serve leaves the access copy untouched.
// - SM-BIN-09: cleanup refused on an unsafe directory after unregistering.
//
// The "loaded definition differs from the file" scenario has no BusyBox
// counterpart: the loaded definition is the init script itself.

/// Write the system access config.
fn write_system_access_config() {
    let peer = iroh::SecretKey::generate().public();
    std::fs::write(
        ACCESS_CONFIG,
        format!(
            "[node]\nsecret_key = {:?}\nrelay_urls = [\"https://relay.invalid\"]\n\n\
             [[services]]\nname = \"alpha\"\nnode_id = \"{peer}\"\nprotocol = \"tcp\"\nport = 39123\n",
            fresh_key()
        ),
    )
    .unwrap();
    own(Path::new(ACCESS_CONFIG), 0);
    mode(Path::new(ACCESS_CONFIG), 0o600);
}

fn place_binary_in(dir: &Path, dir_uid: u32, bin_uid: u32) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    own(dir, dir_uid);
    mode(dir, 0o755);
    let dest = dir.join("iroh-tunnel");
    place_binary(&dest, bin_uid);
    dest
}

fn assert_registers_copy(bin: &Path, dir: &Path) {
    let before = std::fs::read(bin).unwrap();
    let meta = owner_and_mode(bin);
    clear_snapshot();
    let path = format!("{}:/usr/bin:/bin", dir.display());
    let out = install_on_path(&path);
    assert!(out.status.success(), "{}", err(&out));
    assert_eq!(daemon(), MANAGED_COPY, "{}", script());
    assert_eq!(owner_and_mode(Path::new(MANAGED_COPY)), (0, 0o755));
    assert_eq!(owner_and_mode(bin), meta);
    assert_eq!(std::fs::read(bin).unwrap(), before);
    wait_running();
}

#[test]
fn sm_bin_06_root_owned_binary_below_user_owned_directory_gets_a_copy() {
    if !enabled() {
        return;
    }
    let _sandbox = Sandbox::new();
    let alice = Home::new("it91a", "alpha");
    write_system_config("alpha", &fresh_key());
    let dir = user_bin_dir(&alice);
    let bin = place_binary_in(&dir, alice.uid, 0);
    assert_registers_copy(&bin, &dir);
}

#[test]
fn sm_bin_07_cargo_and_homebrew_prefixes_register_a_copy() {
    if !enabled() {
        return;
    }
    let _sandbox = Sandbox::new();
    let alice = Home::new("it91a", "alpha");
    write_system_config("alpha", &fresh_key());

    let cargo = alice.home.join(".cargo/bin");
    std::fs::create_dir_all(&cargo).unwrap();
    own(&alice.home.join(".cargo"), alice.uid);
    let bin = place_binary_in(&cargo, alice.uid, alice.uid);
    assert_registers_copy(&bin, &cargo);
    stop_and_release(&user_path(&alice));
    reset_role(SCRIPT, PIDFILE, "serve");
    let _ = std::fs::remove_dir_all(MANAGED_DIR);

    let brew = Path::new(BREW_PREFIX).join("bin");
    std::fs::create_dir_all(&brew).unwrap();
    for dir in ["/home/linuxbrew", BREW_PREFIX] {
        own(Path::new(dir), alice.uid);
        mode(Path::new(dir), 0o755);
    }
    let bin = place_binary_in(&brew, alice.uid, alice.uid);
    assert_registers_copy(&bin, &brew);
}

#[test]
fn sm_bin_08_uninstalling_serve_keeps_the_access_copy() {
    if !enabled() {
        return;
    }
    let _sandbox = Sandbox::new();
    let alice = Home::new("it91a", "alpha");
    write_system_config("alpha", &fresh_key());
    write_system_access_config();
    place_user_binary(&alice);
    let path = user_path(&alice);

    let out = install_on_path(&path);
    assert!(out.status.success(), "{}", err(&out));
    let out = run(&["access", "service", "install"], &[("PATH", &path)]);
    assert!(out.status.success(), "access: {}", err(&out));
    let access_bytes = std::fs::read(ACCESS_COPY).unwrap();
    let access_meta = std::fs::metadata(ACCESS_COPY).unwrap();
    assert!(Path::new(MANAGED_COPY).exists());

    let pid = pidfile_pid();
    let out = service_verb("uninstall", &path);
    assert!(out.status.success(), "uninstall: {}", err(&out));
    wait_released(&pid);
    assert!(std::fs::symlink_metadata(MANAGED_COPY).is_err());
    assert!(std::fs::symlink_metadata(MANAGED_DIR).is_err());
    assert_eq!(std::fs::read(ACCESS_COPY).unwrap(), access_bytes);
    let after = std::fs::metadata(ACCESS_COPY).unwrap();
    assert_eq!(
        (after.uid(), after.mode() & 0o7777, after.ino()),
        (
            access_meta.uid(),
            access_meta.mode() & 0o7777,
            access_meta.ino()
        )
    );
}

#[test]
fn sm_bin_09_cleanup_on_unsafe_directory_fails_after_unregistering() {
    if !enabled() {
        return;
    }
    let _sandbox = Sandbox::new();
    let alice = Home::new("it91a", "alpha");
    write_system_config("alpha", &fresh_key());
    place_user_binary(&alice);
    let path = user_path(&alice);

    let out = install_on_path(&path);
    assert!(out.status.success(), "{}", err(&out));
    wait_running();
    stop_and_release(&path);
    own(Path::new(MANAGED_DIR), alice.uid);

    let out = service_verb("uninstall", &path);
    let text = err(&out);
    assert_eq!(out.status.code(), Some(3), "{text}");
    assert!(text.contains(MANAGED_DIR), "{text}");
    assert!(
        !Path::new(SCRIPT).exists(),
        "the registration must be gone: {text}"
    );
    assert_eq!(owner_and_mode(Path::new(MANAGED_DIR)), (alice.uid, 0o755));
    assert!(Path::new(MANAGED_COPY).exists(), "the copy must be kept");
}
