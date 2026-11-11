//! #91: install and run a system service across the privilege boundary,
//! driven through the real binary as real root with real user accounts.
//!
//! Acceptance map:
//! - SM-ELEVATED-01 (91.AC1, 91.CL1): root defaults to system scope with no
//!   scope prompt, using the invoking user's paths, never root's home.
//! - SM-PATH-01 (91.AC2): the registration pins the installing user's absolute
//!   config/state paths and the service runs from them.
//! - SM-RETARGET-01 (91.AC2, 91.CL2): retargeting another user's service needs
//!   confirmation; decline leaves registration and both configs unchanged.
//! - SM-UNKNOWN-01 (91.CL2): an unknown user is asked interactively or fails
//!   with guidance; root's home is never substituted.
//! - SM-EDIT-01 (91.CL3): a user's edit applies after an administrator restart;
//!   an invalid edit is reported; ownership and mode are never changed.
//! - SM-TRUST-01 (91.AC2, 91.CL3): an unauthorized writer, a group/world
//!   writable parent, a symlink and a parent swapped for a link make the
//!   service refuse to start, naming the path, owner and mode.
//! - SM-BIN-01 (binary trust): install from a user-owned bin directory
//!   registers a root-owned 0755 managed copy; the original is untouched.
//! - SM-BIN-02: a legacy registration naming a user-writable binary refuses
//!   CLI start (exit 3, no pidfile) while the init script still runs it.
//! - SM-BIN-03: uninstall removes the copy and its directory; a root-owned
//!   binary on `PATH` is registered directly with no copy.
//! - SM-BIN-04: a role directory owned by a user refuses install (exit 3).
//! - SM-BIN-05: a declined retarget writes no managed copy.
//!
//! Needs real root, real accounts and an init system the test may register
//! with, so it is opt-in: `IROH_TUNNEL_E2E_PRIVILEGED=1` in a disposable Linux
//! container without systemd (the BusyBox backend). It adds users `it91a`,
//! `it91b`, `it91x` and removes the registration on exit. The `test-utils`
//! feature disables relay TLS verification: only an unreachable relay is used,
//! never public relays or real secrets.

// The prompts are answered through the root crate's `test-utils` seam
// `IROH_TUNNEL_TEST_INTERACTIVE`, so the file needs `--all-features`.
#![cfg(all(target_os = "linux", feature = "test-utils"))]

use std::io::Write;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_iroh-tunnel");
const SCRIPT: &str = "/etc/init.d/S96iroh-tunnel-serve";
const LOG: &str = "/var/log/iroh-tunnel-serve.log";

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

/// A user's home with `.config/iroh-tunnel/serve.toml` and a state directory.
struct Home {
    name: &'static str,
    uid: u32,
    home: PathBuf,
}

impl Home {
    fn new(name: &'static str, service: &str) -> Self {
        let (uid, home) = ensure_user(name);
        let h = Self { name, uid, home };
        std::fs::create_dir_all(h.config().parent().unwrap()).unwrap();
        std::fs::create_dir_all(h.state()).unwrap();
        h.write_config(service);
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
    fn write_config(&self, service: &str) {
        let key = iroh::SecretKey::generate();
        let path = self.config();
        std::fs::write(
            &path,
            format!(
                "[node]\nsecret_key = {:?}\nrelay_urls = [\"https://relay.invalid\"]\n\n\
                 [[services]]\nname = \"{service}\"\nprotocol = \"tcp\"\nport = 9\n",
                iroh_tunnel_host::config::encode_secret_key(&key)
            ),
        )
        .unwrap();
        own(&path, self.uid);
        mode(&path, 0o600);
    }
    fn status_file(&self) -> PathBuf {
        self.state().join("serve-status.json")
    }
    fn running_services(&self) -> Option<String> {
        std::fs::read_to_string(self.status_file()).ok()
    }
}

fn run(args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut command = Command::new(BIN);
    command
        .args(args)
        .env("HOME", "/root")
        .env_remove("SUDO_USER")
        .env_remove("IROH_TUNNEL_STATE_DIR")
        .env_remove("IROH_TUNNEL_TEST_INTERACTIVE");
    for (k, v) in envs {
        command.env(k, v);
    }
    command.output().unwrap()
}

fn run_with_stdin(args: &[&str], envs: &[(&str, &str)], stdin: &str) -> Output {
    let mut command = Command::new(BIN);
    command
        .args(args)
        .env("HOME", "/root")
        .env_remove("SUDO_USER")
        .env_remove("IROH_TUNNEL_STATE_DIR")
        .env("IROH_TUNNEL_TEST_INTERACTIVE", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in envs {
        command.env(k, v);
    }
    let mut child = command.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn err(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn script() -> String {
    std::fs::read_to_string(SCRIPT).unwrap_or_default()
}

fn wait_for(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if ready() {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    panic!("timeout waiting for {what}");
}

struct Cleanup;
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = Command::new(BIN)
            .args(["serve", "service", "uninstall", "--system"])
            .output();
        let _ = std::fs::remove_file(LOG);
    }
}

fn install(user: &Home) -> Output {
    run(
        &["serve", "service", "install"],
        &[
            ("SUDO_USER", user.name),
            ("IROH_TUNNEL_STATE_DIR", user.state().to_str().unwrap()),
        ],
    )
}

fn tail_log() -> String {
    std::fs::read_to_string(LOG).unwrap_or_default()
}

fn restart() -> Output {
    run(&["serve", "service", "restart", "--system"], &[])
}

fn start_and_wait(user: &Home, what: &str) {
    let _ = std::fs::remove_file(user.status_file());
    let out = restart();
    assert!(out.status.success(), "restart: {}", err(&out));
    wait_for(what, || user.running_services().is_some());
}

/// Restart and require a refusal that never reaches `running`.
fn restart_expecting_refusal(user: &Home) -> String {
    let _ = std::fs::remove_file(user.status_file());
    let _ = std::fs::remove_file(LOG);
    let out = restart();
    assert!(out.status.success(), "restart: {}", err(&out));
    wait_for("the refusal to be logged", || {
        tail_log().contains("permission error")
    });
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        user.running_services().is_none(),
        "the service must not run on an unsafe path"
    );
    tail_log()
}

#[test]
fn system_service_across_the_privilege_boundary() {
    if !enabled() {
        return;
    }
    let _cleanup = Cleanup;
    let alice = Home::new("it91a", "alpha");
    let bob = Home::new("it91b", "bravo");
    let alice_config = alice.config().canonicalize().unwrap();

    // --- SM-UNKNOWN-01: no SUDO_USER, no terminal -> guidance, nothing registered.
    let out = run(&["serve", "service", "install"], &[]);
    let text = err(&out);
    assert_eq!(out.status.code(), Some(2), "{text}");
    assert!(
        text.contains("--config") && text.contains("sudo iroh-tunnel"),
        "{text}"
    );
    assert!(
        !Path::new(SCRIPT).exists(),
        "unknown user must register nothing"
    );
    assert!(
        !script().contains("/root"),
        "root's home must never be substituted"
    );

    // --- SM-ELEVATED-01 + SM-PATH-01: root defaults to system scope, no scope
    // prompt, pinned to alice's absolute paths (HOME=/root is set).
    let out = install(&alice);
    assert!(out.status.success(), "{}", err(&out));
    assert!(
        !err(&out).contains("`user` service"),
        "no scope prompt: {}",
        err(&out)
    );
    let registered = script();
    assert!(
        registered.contains(alice_config.to_str().unwrap()),
        "{registered}"
    );
    assert!(
        registered.contains(alice.state().to_str().unwrap()),
        "{registered}"
    );
    assert!(
        registered.contains(&format!("IROH_TUNNEL_TRUSTED_UID={}", alice.uid)),
        "{registered}"
    );
    assert!(!registered.contains("/root/"), "{registered}");
    wait_for("alice's service to report status", || {
        alice
            .running_services()
            .is_some_and(|s| s.contains("alpha"))
    });
    let before = std::fs::metadata(alice.config()).unwrap();

    // --- SM-EDIT-01: the user edits; the administrator restarts; it applies.
    alice.write_config("alpha2");
    start_and_wait(&alice, "the edited config to be served");
    assert!(alice.running_services().unwrap().contains("alpha2"));
    let after = std::fs::metadata(alice.config()).unwrap();
    assert_eq!((after.uid(), after.mode() & 0o7777), (alice.uid, 0o600));
    let _ = before;
    // An invalid edit is reported and is not served; nothing is changed.
    std::fs::write(alice.config(), "[node]\nnot valid toml = =\n").unwrap();
    let _ = std::fs::remove_file(alice.status_file());
    let _ = std::fs::remove_file(LOG);
    assert!(restart().status.success());
    wait_for("the parse error", || tail_log().contains("failed to parse"));
    assert!(alice.running_services().is_none());
    let broken = std::fs::metadata(alice.config()).unwrap();
    assert_eq!((broken.uid(), broken.mode() & 0o7777), (alice.uid, 0o600));
    alice.write_config("alpha3");

    // --- SM-RETARGET-01: bob's install over alice's service needs a yes.
    let registration = script();
    let alice_config_bytes = std::fs::read(alice.config()).unwrap();
    let bob_config_bytes = std::fs::read(bob.config()).unwrap();
    let bob_state = bob.state();
    let envs = [
        ("SUDO_USER", bob.name),
        ("IROH_TUNNEL_STATE_DIR", bob_state.to_str().unwrap()),
    ];
    for refusal in ["n\n", "\n"] {
        let out = run_with_stdin(&["serve", "service", "install"], &envs, refusal);
        let text = err(&out);
        assert_eq!(out.status.code(), Some(5), "{text}");
        assert!(
            text.contains("Retarget") && text.contains(alice.name),
            "{text}"
        );
        assert_eq!(
            script(),
            registration,
            "decline must not change the service"
        );
    }
    assert_eq!(std::fs::read(alice.config()).unwrap(), alice_config_bytes);
    assert_eq!(std::fs::read(bob.config()).unwrap(), bob_config_bytes);
    // Without a terminal it fails instead of assuming yes.
    let out = run(&["serve", "service", "install"], &envs);
    assert_eq!(out.status.code(), Some(5), "{}", err(&out));
    assert_eq!(script(), registration);
    // Approval retargets and keeps both configs.
    let out = run_with_stdin(&["serve", "service", "install"], &envs, "y\n");
    assert!(out.status.success(), "{}", err(&out));
    let retargeted = script();
    assert!(retargeted.contains(&format!("IROH_TUNNEL_TRUSTED_UID={}", bob.uid)));
    assert!(retargeted.contains(bob.config().canonicalize().unwrap().to_str().unwrap()));
    assert_eq!(std::fs::read(alice.config()).unwrap(), alice_config_bytes);
    assert_eq!(std::fs::read(bob.config()).unwrap(), bob_config_bytes);
    // Same user again is not a retarget.
    let out = run(&["serve", "service", "install"], &envs);
    assert!(out.status.success(), "{}", err(&out));

    // --- SM-TRUST-01 against bob's running service.
    wait_for("bob's service", || bob.running_services().is_some());
    let config = bob.config();
    let dir = config.parent().unwrap().to_path_buf();

    // Unauthorized writer: group/world-writable config file.
    mode(&config, 0o666);
    let log = restart_expecting_refusal(&bob);
    assert!(
        log.contains(config.to_str().unwrap()) && log.contains("0666"),
        "{log}"
    );
    mode(&config, 0o600);

    // Another user owns the file.
    let (mallory, _) = ensure_user("it91x");
    own(&config, mallory);
    let log = restart_expecting_refusal(&bob);
    assert!(log.contains(&format!("uid {mallory}")), "{log}");
    own(&config, bob.uid);

    // Hostile parent: a directory the attacker can write to.
    for parent in [dir.clone(), bob.home.join(".config"), bob.home.clone()] {
        mode(&parent, 0o777);
        let log = restart_expecting_refusal(&bob);
        assert!(log.contains(parent.to_str().unwrap()), "{log}");
        mode(&parent, 0o755);
    }

    // Parent owned by an untrusted user (swap by the attacker's directory).
    own(&dir, mallory);
    let log = restart_expecting_refusal(&bob);
    assert!(log.contains("untrusted"), "{log}");
    own(&dir, bob.uid);

    // Symlink in place of the file.
    let real = bob.home.join("elsewhere.toml");
    std::fs::rename(&config, &real).unwrap();
    std::os::unix::fs::symlink(&real, &config).unwrap();
    let log = restart_expecting_refusal(&bob);
    assert!(log.contains("symlink"), "{log}");
    std::fs::remove_file(&config).unwrap();
    std::fs::rename(&real, &config).unwrap();

    // Parent directory swapped for a symlink.
    let moved = bob.home.join(".config-real");
    let cfg_parent = bob.home.join(".config");
    std::fs::rename(&cfg_parent, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, &cfg_parent).unwrap();
    let log = restart_expecting_refusal(&bob);
    assert!(log.contains("symlink"), "{log}");
    std::fs::remove_file(&cfg_parent).unwrap();
    std::fs::rename(&moved, &cfg_parent).unwrap();

    // Nothing above changed an owner or mode: the service never chown/chmods.
    let meta = std::fs::metadata(&config).unwrap();
    assert_eq!((meta.uid(), meta.mode() & 0o7777), (bob.uid, 0o600));

    // Once the unsafe condition is removed the service runs again.
    start_and_wait(&bob, "bob's service after the fix");

    // --- RO-03: a real kill leaves files behind; they must not block restart.
    let pid = std::fs::read_to_string("/var/run/iroh-tunnel-serve.pid")
        .unwrap()
        .trim()
        .to_owned();
    assert!(sh(&["kill", "-9", &pid]).status.success());
    wait_for("the killed process to exit", || {
        // Without an init to reap it, a killed child stays a zombie (`Z`).
        std::fs::read_to_string(format!("/proc/{pid}/stat")).map_or(true, |stat| {
            stat.rsplit(") ").next().is_some_and(|r| r.starts_with('Z'))
        })
    });
    assert!(
        bob.status_file().exists(),
        "a kill -9 leaves the snapshot behind"
    );
    std::fs::write(bob.state().join("serve-status.json.tmp.1.1"), "junk").unwrap();
    let out = restart();
    assert!(out.status.success(), "restart: {}", err(&out));
    wait_for("a fresh snapshot after the crash", || {
        bob.running_services()
            .is_some_and(|s| !s.contains(&format!("\"pid\": {pid}")))
    });

    // --- Root never writes through a link the user plants in the state dir.
    let victim = Path::new("/root/it91-victim");
    std::fs::write(victim, "keep").unwrap();
    let _ = std::fs::remove_file(bob.status_file());
    std::os::unix::fs::symlink(victim, bob.status_file()).unwrap();
    let out = restart();
    assert!(out.status.success(), "restart: {}", err(&out));
    wait_for("the snapshot to replace the planted link", || {
        std::fs::symlink_metadata(bob.status_file()).is_ok_and(|m| m.file_type().is_file())
    });
    assert_eq!(std::fs::read_to_string(victim).unwrap(), "keep");
    let _ = std::fs::remove_file(victim);

    // --- SM-04 fail closed: root under a service manager with no pin (a
    // registration from before the policy) is refused; a foreground root run
    // of the same config is not a service and is unaffected.
    let out = run(&["serve", "service", "stop", "--system"], &[]);
    assert!(out.status.success(), "stop: {}", err(&out));
    let legacy = Command::new(BIN)
        .args(["serve", "run", "--config", bob.config().to_str().unwrap()])
        .env("HOME", "/root")
        .env("INVOCATION_ID", "legacy-unit")
        .env_remove("IROH_TUNNEL_TRUSTED_UID")
        .env("IROH_TUNNEL_STATE_DIR", "/tmp/it91-legacy-state")
        .output()
        .unwrap();
    assert_eq!(legacy.status.code(), Some(3), "{}", err(&legacy));
    assert!(
        err(&legacy).contains("service install --system"),
        "{}",
        err(&legacy)
    );

    // --- A symlinked home is resolved once at install (SM-04), not refused later.
    let real = PathBuf::from("/home/it91b-real");
    let _ = std::fs::remove_dir_all(&real);
    std::fs::rename(&bob.home, &real).unwrap();
    std::os::unix::fs::symlink(&real, &bob.home).unwrap();
    let out = run(
        &["serve", "service", "install"],
        &[
            ("SUDO_USER", bob.name),
            ("IROH_TUNNEL_STATE_DIR", bob_state.to_str().unwrap()),
        ],
    );
    assert!(out.status.success(), "{}", err(&out));
    assert!(script().contains("/home/it91b-real/"), "{}", script());
    wait_for("bob's service via the resolved paths", || {
        std::fs::metadata(real.join(".local/state/iroh-tunnel/serve-status.json")).is_ok()
    });
}

// --- System binary trust (SM-BIN-*) ---------------------------------------

const PIDFILE: &str = "/var/run/iroh-tunnel-serve.pid";
const MANAGED_DIR: &str = "/var/lib/iroh-tunnel/bin/serve";
const MANAGED_COPY: &str = "/var/lib/iroh-tunnel/bin/serve/iroh-tunnel";
const SYSTEM_BIN: &str = "/usr/local/bin/iroh-tunnel";
const SYSTEM_PATH: &str = "/usr/local/bin:/usr/bin:/bin";
const NOTE: &str = "Registered a root-owned copy of";

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

fn install_on_path(user: &Home, path: &str) -> Output {
    run(
        &["serve", "service", "install"],
        &[
            ("SUDO_USER", user.name),
            ("IROH_TUNNEL_STATE_DIR", user.state().to_str().unwrap()),
            ("PATH", path),
        ],
    )
}

fn service_verb(verb: &str, path: &str) -> Output {
    run(&["serve", "service", verb, "--system"], &[("PATH", path)])
}

/// The binary the init script runs.
fn daemon() -> String {
    script()
        .lines()
        .find_map(|l| l.strip_prefix("DAEMON=\""))
        .and_then(|l| l.strip_suffix('"'))
        .map(str::to_owned)
        .unwrap_or_default()
}

fn exited(pid: &str) -> bool {
    // Without an init to reap it, an exited child stays a zombie (`Z`).
    std::fs::read_to_string(format!("/proc/{pid}/stat")).map_or(true, |stat| {
        stat.rsplit(") ").next().is_some_and(|r| r.starts_with('Z'))
    })
}

/// Wait until the stopped service's process is gone, which releases its role
/// lock, and its pidfile is removed.
fn wait_released(pid: &str) {
    wait_for("the service process to exit", || exited(pid));
    wait_for("the pidfile to clear", || !Path::new(PIDFILE).exists());
}

/// `service stop`, then wait for the release of the pidfile and role lock.
fn stop_and_release(path: &str) {
    let pid = std::fs::read_to_string(PIDFILE)
        .unwrap_or_default()
        .trim()
        .to_owned();
    let out = service_verb("stop", path);
    assert!(out.status.success(), "stop: {}", err(&out));
    wait_released(&pid);
}

/// Whether `pid` is a live process (a zombie counts as gone).
fn alive(pid: &str) -> bool {
    !exited(pid)
}

/// The start time (field 22 of `/proc/<pid>/stat`) of `pid`, if it exists.
fn start_time(pid: &str) -> Option<String> {
    if pid.is_empty() {
        return None;
    }
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // Fields after the parenthesised command start at field 3.
    stat.rsplit(") ")
        .next()?
        .split_whitespace()
        .nth(19)
        .map(str::to_owned)
}

/// The pid the service wrote into its status snapshot, if readable.
fn snapshot_pid(user: &Home) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(&user.running_services()?).ok()?;
    Some(json.get("pid")?.as_u64()?.to_string())
}

/// Drop a snapshot left by an earlier process, so readiness can only come
/// from the process about to start. Call it once the predecessor is gone.
fn clear_snapshot(user: &Home) {
    let _ = std::fs::remove_file(user.status_file());
}

/// Wait until the live process in the pidfile has written its own snapshot:
/// a snapshot of a previous (even killed) process never satisfies this.
fn wait_running(user: &Home) {
    wait_for("the live service to report its own status", || {
        let pid = std::fs::read_to_string(PIDFILE)
            .map(|p| p.trim().to_owned())
            .unwrap_or_default();
        !pid.is_empty() && alive(&pid) && snapshot_pid(user).as_deref() == Some(pid.as_str())
    });
}

/// Start from nothing and put everything these tests touch back on drop.
struct BinSandbox;

impl BinSandbox {
    fn new() -> Self {
        reset_binary_state();
        Self
    }
}

impl Drop for BinSandbox {
    fn drop(&mut self) {
        reset_binary_state();
    }
}

/// Remove the registration, the running service and every binary copy the
/// SM-BIN tests create, whatever state a previous test left.
fn reset_binary_state() {
    let pid = std::fs::read_to_string(PIDFILE)
        .unwrap_or_default()
        .trim()
        .to_owned();
    // Identify the process by pid and start time, to reduce the risk of
    // killing or waiting on a reused pid.
    let started = start_time(&pid);
    let same_alive = || started.is_some() && start_time(&pid) == started && alive(&pid);
    if Path::new(SCRIPT).exists() {
        let _ = Command::new("sh").args([SCRIPT, "stop"]).output();
    }
    if same_alive() {
        // A service that ignored `stop` must not outlive its binaries.
        if let Ok(n) = pid.parse::<i32>() {
            // SAFETY: kill(2) takes plain integers and has no memory effects.
            unsafe { libc::kill(n, libc::SIGKILL) };
        }
    }
    // Uninstall may delete the managed copy, so the process must be gone first.
    wait_for("the previous service process to exit", || !same_alive());
    if Path::new(SCRIPT).exists() {
        let _ = Command::new(BIN)
            .args(["serve", "service", "uninstall", "--system"])
            .output();
        let _ = std::fs::remove_file(SCRIPT);
    }
    let _ = std::fs::remove_file(PIDFILE);
    let _ = std::fs::remove_file(LOG);
    let _ = std::fs::remove_dir_all(MANAGED_DIR);
    let _ = std::fs::remove_file(SYSTEM_BIN);
    for name in ["it91a", "it91b"] {
        let _ = std::fs::remove_dir_all(format!("/home/{name}/bin"));
    }
}

#[test]
fn sm_bin_01_user_bin_dir_registers_managed_copy() {
    if !enabled() {
        return;
    }
    let _sandbox = BinSandbox::new();
    let alice = Home::new("it91a", "alpha");
    place_user_binary(&alice);
    let original = std::fs::metadata(user_bin(&alice)).unwrap();
    let original_bytes = std::fs::read(user_bin(&alice)).unwrap();

    clear_snapshot(&alice);
    let out = install_on_path(&alice, &user_path(&alice));
    assert!(out.status.success(), "{}", err(&out));

    assert_eq!(daemon(), MANAGED_COPY, "{}", script());
    let copy = std::fs::metadata(MANAGED_COPY).unwrap();
    assert_eq!((copy.uid(), copy.mode() & 0o7777), (0, 0o755));
    assert_eq!(
        std::fs::read(MANAGED_COPY).unwrap(),
        std::fs::read(BIN).unwrap()
    );
    let after = std::fs::metadata(user_bin(&alice)).unwrap();
    assert_eq!(
        (after.uid(), after.mode() & 0o7777),
        (original.uid(), original.mode() & 0o7777)
    );
    assert_eq!(std::fs::read(user_bin(&alice)).unwrap(), original_bytes);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(NOTE)
            // The source is the running executable the install copied from.
            && stdout.contains(BIN)
            && stdout.contains(MANAGED_COPY)
            && stdout.contains("service install --system"),
        "{stdout}"
    );
    wait_running(&alice);
}

#[test]
fn sm_bin_02_legacy_unsafe_daemon_refuses_start() {
    if !enabled() {
        return;
    }
    let _sandbox = BinSandbox::new();
    let alice = Home::new("it91a", "alpha");
    place_user_binary(&alice);
    let path = user_path(&alice);
    let user_binary = user_bin(&alice);
    let user_binary = user_binary.to_str().unwrap();

    clear_snapshot(&alice);
    let out = install_on_path(&alice, &path);
    assert!(out.status.success(), "{}", err(&out));
    wait_running(&alice);
    stop_and_release(&path);

    // A registration from before this change: the user's own binary.
    let legacy = script().replace(&daemon(), user_binary);
    std::fs::write(SCRIPT, legacy).unwrap();
    assert_eq!(daemon(), user_binary);
    let _ = std::fs::remove_file(alice.status_file());
    let out = service_verb("start", &path);
    let text = err(&out);
    assert_eq!(out.status.code(), Some(3), "{text}");
    // The refusal names the registered binary and the reinstall command.
    assert!(
        text.contains(user_binary) && text.contains("service install --system"),
        "{text}"
    );
    assert!(!Path::new(PIDFILE).exists(), "a refused start must not run");

    // The init system starts it with no self-check, as at boot.
    clear_snapshot(&alice);
    let status = Command::new("sh")
        .args([SCRIPT, "start"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    wait_running(&alice);
    stop_and_release(&path);

    // Re-installing registers the managed copy, and start works again.
    clear_snapshot(&alice);
    let out = install_on_path(&alice, &path);
    assert!(out.status.success(), "{}", err(&out));
    assert_eq!(daemon(), MANAGED_COPY);
    wait_running(&alice);
    stop_and_release(&path);
    let _ = std::fs::remove_file(alice.status_file());
    let out = service_verb("start", &path);
    assert!(out.status.success(), "start: {}", err(&out));
    wait_running(&alice);
}

#[test]
fn sm_bin_03_root_path_direct_and_uninstall_cleans() {
    if !enabled() {
        return;
    }
    let _sandbox = BinSandbox::new();
    let alice = Home::new("it91a", "alpha");
    place_user_binary(&alice);
    let path = user_path(&alice);

    clear_snapshot(&alice);
    let out = install_on_path(&alice, &path);
    assert!(out.status.success(), "{}", err(&out));
    assert!(Path::new(MANAGED_COPY).exists());
    wait_running(&alice);
    let pid = std::fs::read_to_string(PIDFILE).unwrap().trim().to_owned();

    let out = service_verb("uninstall", &path);
    assert!(out.status.success(), "uninstall: {}", err(&out));
    assert!(std::fs::symlink_metadata(MANAGED_COPY).is_err());
    assert!(std::fs::symlink_metadata(MANAGED_DIR).is_err());
    wait_released(&pid);

    // A root-owned binary on PATH is registered directly.
    place_system_binary();
    let out = install_on_path(&alice, SYSTEM_PATH);
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
    let _sandbox = BinSandbox::new();
    let alice = Home::new("it91a", "alpha");
    place_user_binary(&alice);
    std::fs::create_dir_all(MANAGED_DIR).unwrap();
    own(Path::new(MANAGED_DIR), alice.uid);
    mode(Path::new(MANAGED_DIR), 0o755);

    let out = install_on_path(&alice, &user_path(&alice));
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
    let dir = std::fs::metadata(MANAGED_DIR).unwrap();
    assert_eq!((dir.uid(), dir.mode() & 0o7777), (alice.uid, 0o755));
}

#[test]
fn sm_bin_05_declined_retarget_writes_no_copy() {
    if !enabled() {
        return;
    }
    let _sandbox = BinSandbox::new();
    let alice = Home::new("it91a", "alpha");
    let bob = Home::new("it91b", "bravo");
    place_system_binary();
    place_user_binary(&bob);

    let out = install_on_path(&alice, SYSTEM_PATH);
    assert!(out.status.success(), "{}", err(&out));
    assert_eq!(daemon(), SYSTEM_BIN);
    let registration = script();

    let bob_state = bob.state();
    let path = user_path(&bob);
    let out = run_with_stdin(
        &["serve", "service", "install"],
        &[
            ("SUDO_USER", bob.name),
            ("IROH_TUNNEL_STATE_DIR", bob_state.to_str().unwrap()),
            ("PATH", &path),
        ],
        "n\n",
    );
    assert_eq!(out.status.code(), Some(5), "{}", err(&out));
    assert!(
        std::fs::symlink_metadata(MANAGED_DIR).is_err(),
        "a declined retarget must write no copy"
    );
    assert_eq!(script(), registration);
}
