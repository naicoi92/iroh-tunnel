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
//!
//! Needs real root, real accounts and an init system the test may register
//! with, so it is opt-in: `IROH_TUNNEL_E2E_PRIVILEGED=1` in a disposable Linux
//! container without systemd (the BusyBox backend). It adds users `it91a`,
//! `it91b`, `it91x` and removes the registration on exit. The `test-utils`
//! feature disables relay TLS verification: only an unreachable relay is used,
//! never public relays or real secrets.

#![cfg(target_os = "linux")]

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
