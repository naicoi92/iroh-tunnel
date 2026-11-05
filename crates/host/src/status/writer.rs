//! Atomic writer and cleaner of one role's status file (private
//! infrastructure; the runtime drives it).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use serde::Deserialize;

use super::schema::StatusPayload;
use crate::role::Role;

/// Pre-rename serve file name, removed best-effort on the first successful
/// serve save of the renamed file so upgraded nodes don't leave a stale
/// snapshot that tooling still pointed at the old name would read silently.
const LEGACY_STATUS_FILE_NAME: &str = "status.json";

/// Environment variable overriding the status files' directory entirely
/// (advanced/testing seam; empty is treated as unset). Shared by both
/// roles — one directory, two role-scoped file names.
const ENV_STATE_DIR: &str = "IROH_TUNNEL_STATE_DIR";

/// Atomic writer for one role's status file.
///
/// Owns the parts shared by both roles so they cannot drift: the
/// temp+fsync+rename save, the `IROH_TUNNEL_STATE_DIR` override, and the
/// injected-state-dir testing seam. Role differences reduce to the file
/// name via [`StatusWriter::serve`] / [`StatusWriter::access`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct StatusWriter(Role);

/// Monotonic per-process counter making each save's temp file name unique
/// (see [`StatusWriter::save_to`]).
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

impl StatusWriter {
    /// Writer targeting `role`'s status file.
    pub(crate) const fn new(role: Role) -> Self {
        Self(role)
    }

    /// Writer for the serve role (`serve-status.json`).
    pub(crate) const fn serve() -> Self {
        Self(Role::Serve)
    }

    /// Writer for the access role (`access-status.json`, issue #59).
    pub(crate) const fn access() -> Self {
        Self(Role::Access)
    }

    /// The role this writer targets.
    #[cfg(test)]
    pub(crate) fn role(&self) -> Role {
        self.0
    }

    /// The file name this writer saves under.
    pub(crate) fn file_name(&self) -> &'static str {
        self.0.status_file_name()
    }

    /// Resolve this role's status file path under the OS state directory.
    pub(crate) fn path(&self) -> Result<PathBuf> {
        self.path_with(std::env::var_os(ENV_STATE_DIR).as_deref())
    }

    /// Pure core of [`StatusWriter::path`]: the path as a function of an
    /// optional env-override value, kept separate so tests never mutate
    /// process-global state.
    ///
    /// The `IROH_TUNNEL_STATE_DIR` environment variable, when set to a
    /// non-empty value, replaces the resolved base dir *entirely* — the file
    /// lands directly in it (no `iroh-tunnel` subpath). This is an
    /// advanced/testing seam: it lets integration tests point a role
    /// instance at a tempdir without touching the real state dir, and lets
    /// packaging relocate the file. An empty value is treated as unset
    /// (standard env-var semantics) so a stray `IROH_TUNNEL_STATE_DIR=`
    /// cannot make the path relative to the current working directory.
    pub(crate) fn path_with(&self, state_dir_override: Option<&OsStr>) -> Result<PathBuf> {
        if let Some(dir) = state_dir_override.filter(|dir| !dir.is_empty()) {
            // Absolutize so a relative override (`IROH_TUNNEL_STATE_DIR=.`)
            // is resolved against the CWD at resolution time. Note this runs
            // per save — a process that changes its CWD would change the
            // target dir; pin the resolved path once if that ever becomes a
            // concern.
            let dir = std::path::absolute(dir).with_context(|| {
                format!("invalid {ENV_STATE_DIR}: {}", Path::new(dir).display())
            })?;
            return Ok(dir.join(self.file_name()));
        }
        Ok(crate::paths::default_state_dir()?.join(self.file_name()))
    }

    /// Write `value` under the env-aware default path for this role.
    ///
    /// Convenience entry point for production callers — resolves
    /// [`Self::path`] and delegates to [`Self::save_to`]. Returns the path
    /// written.
    pub(crate) fn save(&self, value: &StatusPayload) -> Result<PathBuf> {
        // Compute the parent dir (the OS state dir + "iroh-tunnel"), so
        // save_to can stay dir-focused and create it if missing.
        let path = self.path()?;
        let dir = path
            .parent()
            .context("status file path has no parent directory")?
            .to_path_buf();
        self.save_to(&dir, value).map(|_| path)
    }

    /// Write `value` honoring an optional injected state dir: `Some(dir)`
    /// writes `<dir>/<file_name>` directly; `None` uses [`Self::save`]
    /// (env-aware default path).
    ///
    /// The seam `testing::run_unowned` entry point feeds: production passes `None`, integration tests inject a tempdir
    /// without mutating process-global environment.
    pub(crate) fn save_with_state_dir(
        &self,
        state_dir: Option<&Path>,
        value: &StatusPayload,
    ) -> Result<PathBuf> {
        match state_dir {
            Some(dir) => self.save_to(dir, value),
            None => self.save(value),
        }
    }

    /// The path [`Self::save_with_state_dir`] writes to for this state-dir
    /// choice: `Some(dir)` → `<dir>/<file_name>`, `None` → the env-aware
    /// default.
    ///
    /// The read/remove counterpart of the save side, for callers that clean
    /// the file up on graceful shutdown — the flush task owns the writer,
    /// so the run loop resolves the same path up front instead of keeping a
    /// second handle to it.
    pub(crate) fn path_for_state_dir(&self, state_dir: Option<&Path>) -> Result<PathBuf> {
        match state_dir {
            Some(dir) => Ok(dir.join(self.file_name())),
            None => self.path(),
        }
    }

    /// Write `value` as `<dir>/<file_name>`, creating `dir` if needed.
    ///
    /// Atomic: the JSON is written to a sibling temp file
    /// (`<file_name>.tmp.<pid>.<n>`) and renamed into place, so a concurrent
    /// reader never observes a half-written file. The temp name is unique
    /// per save so two processes sharing a state dir cannot interleave their
    /// temp writes. Returns the path written.
    ///
    /// This is the testable core — production callers use [`Self::save`] or
    /// [`Self::save_with_state_dir`]; tests inject a `tempfile::tempdir()`
    /// here.
    pub(crate) fn save_to(&self, dir: &Path, value: &StatusPayload) -> Result<PathBuf> {
        // The payload enum re-ties schema to role at this boundary. A
        // mismatched writer/payload pair is a programmer error — bailed in
        // EVERY build profile (not just debug), because silently writing
        // one role's schema under the other's file name is a corrupting
        // failure no test fleet may paper over.
        if self.0 != value.role() {
            anyhow::bail!(
                "status writer role ({}) does not match payload role ({})",
                self.0.name(),
                value.role().name()
            );
        }
        let path = dir.join(self.file_name());
        // Serialize the INNER schema — the enum is a type-level tie, not a
        // JSON tag.
        let content = match value {
            StatusPayload::Serve(file) => serde_json::to_string_pretty(file),
            StatusPayload::Access(file) => serde_json::to_string_pretty(file),
        }
        .context("failed to encode status JSON")?;

        // Under a system service root writes into a user-owned directory:
        // go through a verified handle instead of paths (SM-04).
        #[cfg(unix)]
        if let Some(uid) = crate::service::trust::trusted_uid_from_env()? {
            let temp = format!(
                "{}.tmp.{}.{}",
                self.file_name(),
                std::process::id(),
                TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
            );
            crate::service::trust::publish(dir, uid, self.file_name(), &temp, content.as_bytes())?;
            // Best-effort, like the path-based branch below.
            let _ = crate::service::trust::remove_in(dir, uid, LEGACY_STATUS_FILE_NAME);
            return Ok(path);
        }
        std::fs::create_dir_all(dir)
            .with_context(|| format!("failed to create status dir: {}", dir.display()))?;
        // Write to a temp file in the same directory, then rename — atomic
        // on POSIX, and on Windows for same-volume same-directory renames.
        // `<name>.tmp.<pid>.<counter>` keeps every save's temp file unique
        // (two processes can share an injected state dir) while the rename
        // stays single-file atomic.
        let temp = dir.join(format!(
            "{}.tmp.{}.{}",
            self.file_name(),
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        // Write + fsync the temp file before renaming: the atomic-rename
        // guarantee below covers concurrent readers, and the fsync covers
        // durability — without it a crash right after the rename can leave
        // an empty destination. One save per change (throttled by the flush
        // loop), so the cost is negligible.
        let mut file = std::fs::File::create(&temp)
            .with_context(|| format!("failed to create status temp file: {}", temp.display()))?;
        std::io::Write::write_all(&mut file, content.as_bytes())
            .and_then(|()| file.sync_all())
            .with_context(|| format!("failed to write status file: {}", temp.display()))?;
        // If rename fails, the temp file is stale — clean it up so repeated
        // failures don't accumulate temp files on disk. The original rename
        // error is still what we return; a cleanup failure is best-effort.
        if let Err(e) = std::fs::rename(&temp, &path) {
            let _ = std::fs::remove_file(&temp);
            return Err(e)
                .with_context(|| format!("failed to finalize status file: {}", path.display()));
        }
        // Best-effort removal of the pre-rename legacy serve file, on every
        // successful save of EITHER role: both roles share one state
        // directory, so an access-only host upgrading from the pre-rename
        // era must clean the stale file too. Tooling pointed at the old
        // name would otherwise read a stale snapshot silently (worse than
        // an error); failure to remove is ignored.
        let _ = std::fs::remove_file(dir.join(LEGACY_STATUS_FILE_NAME));
        Ok(path)
    }
}

/// Just the `pid` field of a status file — the minimal probe
/// [`remove_own_status_file`] needs, so the ownership check works against
/// both roles' schemas (unknown fields are ignored by serde).
#[derive(Deserialize)]
struct StatusFilePid {
    pid: u32,
}

/// Remove `writer`'s status file on graceful shutdown — but only while it
/// is still OURS.
///
/// Two processes can share one state dir (the injected seam allows it, and
/// nothing else about the layout forbids it). The flush loop only rewrites
/// when the rendered snapshot CHANGES, so a co-resident idle instance would
/// not necessarily recreate its file after a blind removal — `<role>
/// status` would then report "not running" for a live role until its next
/// real change. The file's own `pid` field decides ownership: remove only
/// when it matches the current process. A missing file (never written),
/// unreadable file, or one that fails to parse is skipped — nothing to
/// clean, or not ours to judge.
pub(crate) async fn remove_own_status_file(writer: StatusWriter, state_dir: Option<&Path>) {
    let state_dir = state_dir.map(Path::to_path_buf);
    // Blocking filesystem work stays off the async workers (RS-07).
    if let Err(error) = tokio::task::spawn_blocking(move || {
        remove_own_status_file_blocking(writer, state_dir.as_deref())
    })
    .await
    {
        tracing::debug!("status cleanup task failed: {error}");
    }
}

/// Blocking twin of [`remove_own_status_file`] for the forced-exit path in
/// `src/main.rs`, where the async runtime is no longer driving cleanup.
/// Resolves the env-aware default directory, like a normal graceful stop.
pub(crate) fn remove_own_status_file_for_exit(role: Role) {
    remove_own_status_file_blocking(StatusWriter::new(role), None);
}

fn remove_own_status_file_blocking(writer: StatusWriter, state_dir: Option<&Path>) {
    let Ok(path) = writer.path_for_state_dir(state_dir) else {
        return;
    };
    #[cfg(unix)]
    if let Ok(Some(uid)) = crate::service::trust::trusted_uid_from_env() {
        use crate::service::trust::{read_in, remove_in};
        let (Some(dir), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str()))
        else {
            return;
        };
        let ours = read_in(dir, uid, name).is_some_and(|body| {
            matches!(
                serde_json::from_str::<StatusFilePid>(&body),
                Ok(probe) if probe.pid == std::process::id()
            )
        });
        if ours {
            if let Err(e) = remove_in(dir, uid, name) {
                tracing::debug!("failed to remove status file: {e:#}");
            }
        }
        return;
    }
    let owns = match std::fs::read_to_string(&path) {
        Ok(body) => {
            matches!(
                serde_json::from_str::<StatusFilePid>(&body),
                Ok(probe) if probe.pid == std::process::id()
            )
        }
        Err(_) => false,
    };
    if !owns {
        return;
    }
    if let Err(e) = std::fs::remove_file(&path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::debug!("failed to remove status file: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::{sample_access_status, sample_status};
    use super::super::schema::{AccessStatusFile, StatusFile};
    use super::*;

    /// [`sample_status`] with a chosen top-level `pid` for the ownership
    /// tests.
    fn sample_status_with_pid(pid: u32) -> StatusFile {
        let mut status = sample_status();
        status.pid = pid;
        status
    }

    /// [`sample_access_status`] with a chosen top-level `pid`.
    fn sample_access_status_with_pid(pid: u32) -> AccessStatusFile {
        let mut status = sample_access_status();
        status.pid = pid;
        status
    }

    /// A stand-in entry for any leftover temp file under `dir` (they are
    /// named `<prefix>.<pid>.<n>`).
    fn leftover_temp_files(dir: &Path, prefix: &str) -> std::path::PathBuf {
        // Any existing entry starting with the temp prefix counts; used with
        // `.exists()` so the caller can assert absence.
        dir.read_dir()
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| e.file_name().to_string_lossy().starts_with(prefix))
            .map(|e| e.path())
            .unwrap_or_else(|| dir.join(format!("{prefix}.none")))
    }

    #[test]
    fn writer_selects_the_role_scoped_file_name() {
        assert_eq!(StatusWriter::serve().file_name(), "serve-status.json");
        assert_eq!(StatusWriter::access().file_name(), "access-status.json");
        assert_eq!(StatusWriter::serve().role(), Role::Serve);
        assert_eq!(StatusWriter::access().role(), Role::Access);
    }

    #[test]
    fn save_to_writes_serve_json_file_into_injected_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let status = sample_status();
        let written = StatusWriter::serve()
            .save_to(tmp.path(), &StatusPayload::Serve(status))
            .unwrap();

        // Path is `<tmp>/serve-status.json`.
        assert_eq!(written, tmp.path().join("serve-status.json"));
        assert!(written.exists(), "status file should exist after save_to");

        // Contents are valid JSON with the expected shape.
        let body = std::fs::read_to_string(&written).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(&body).expect("status file should be valid JSON");
        assert_eq!(parsed["node_id"], "abc123");
        assert_eq!(parsed["pid"], 42);
        assert_eq!(parsed["services"][0]["name"], "echo");
    }

    #[test]
    fn save_to_writes_access_json_file_into_injected_dir() {
        // Issue #59: the access writer targets its own file name in the same
        // injected dir — and must NOT touch the serve file.
        let tmp = tempfile::tempdir().unwrap();
        let status = sample_access_status();
        let written = StatusWriter::access()
            .save_to(tmp.path(), &StatusPayload::Access(status))
            .unwrap();

        assert_eq!(written, tmp.path().join("access-status.json"));
        assert!(
            written.exists(),
            "access status file should exist after save_to"
        );
        assert!(
            !tmp.path().join("serve-status.json").exists(),
            "access save must not create the serve file"
        );

        let body = std::fs::read_to_string(&written).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(&body).expect("status file should be valid JSON");
        assert_eq!(parsed["node_id"], "acc789");
        assert_eq!(parsed["services"][0]["peer"], "peer456");
    }

    #[test]
    fn save_to_creates_missing_parent_dir() {
        // A nested path that doesn't exist yet — save_to must mkdir -p it.
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("a").join("b").join("c");
        let status = sample_status();

        let written = StatusWriter::serve()
            .save_to(&nested, &StatusPayload::Serve(status))
            .unwrap();
        assert!(written.exists(), "status file should exist after save_to");
        assert!(nested.is_dir(), "parent dir should have been created");
    }

    #[test]
    fn save_to_uses_atomic_rename_temp_file_gone() {
        // After a successful save_to, no sibling temp file may linger —
        // atomic rename consumes it. Temp names carry a unique
        // `.tmp.<pid>.<n>` suffix, so scan for the prefix.
        let tmp = tempfile::tempdir().unwrap();
        let status = sample_status();
        StatusWriter::serve()
            .save_to(tmp.path(), &StatusPayload::Serve(status))
            .unwrap();

        assert!(
            !leftover_temp_files(tmp.path(), "serve-status.json.tmp").exists(),
            "serve temp file leaked after rename"
        );
        StatusWriter::access()
            .save_to(tmp.path(), &StatusPayload::Access(sample_access_status()))
            .unwrap();
        assert!(
            !leftover_temp_files(tmp.path(), "access-status.json.tmp").exists(),
            "access temp file leaked after rename"
        );
    }

    #[test]
    fn save_to_overwrites_existing_status() {
        // A second save_to on the same dir replaces the previous file
        // atomically — no stale contents.
        let tmp = tempfile::tempdir().unwrap();
        let mut status = sample_status();
        StatusWriter::serve()
            .save_to(tmp.path(), &StatusPayload::Serve(status.clone()))
            .unwrap();

        status.node_id = "xyz789".to_string();
        StatusWriter::serve()
            .save_to(tmp.path(), &StatusPayload::Serve(status))
            .unwrap();

        let body = std::fs::read_to_string(tmp.path().join("serve-status.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["node_id"], "xyz789");
    }

    #[test]
    fn save_to_cleans_up_temp_file_when_rename_fails() {
        // Drive rename failure by making the destination path unwritable:
        // create a *directory* at serve-status.json's slot so the rename
        // target is occupied by an incompatible entry. This forces rename()
        // to fail on most platforms; if it doesn't fail, the test asserts
        // the post-condition (no stale temp) only when the rename actually
        // failed, so it can't false-positive on a platform where rename
        // overwrites a directory.
        let tmp = tempfile::tempdir().unwrap();
        // Block the destination: a directory at serve-status.json.
        std::fs::create_dir_all(tmp.path().join("serve-status.json")).unwrap();

        let status = sample_status();
        let res = StatusWriter::serve().save_to(tmp.path(), &StatusPayload::Serve(status));

        if res.is_err() {
            // The load-bearing assertion: the temp file must not linger
            // after a rename failure.
            assert!(
                !leftover_temp_files(tmp.path(), "serve-status.json.tmp").exists(),
                "temp file leaked after rename failure"
            );
        }
        // If rename somehow succeeded (platform-specific), the temp file is
        // gone anyway because the rename consumed it — no extra assertion.
    }

    #[test]
    fn status_file_path_override_lands_directly_in_the_dir() {
        // Pure resolver: an override replaces the whole directory — the file
        // lands directly in it, no `iroh-tunnel` subpath. Both roles share
        // the resolver and differ only in the trailing file name.
        let path = StatusWriter::serve()
            .path_with(Some(OsStr::new("/tmp/status-override")))
            .unwrap();
        // Suffix assertions instead of exact-equality: `std::path::absolute`
        // yields a drive-rooted path with backslashes on Windows, so an
        // exact `PathBuf` compare would fail on a supported platform. What
        // the test actually proves: the override replaces the dir entirely
        // (absolute, no `iroh-tunnel` subpath) and keeps the file name.
        assert!(path.is_absolute());
        assert!(
            path.ends_with(std::path::Path::new("status-override").join("serve-status.json")),
            "override must replace the dir entirely, got {path:?}"
        );
        let access_path = StatusWriter::access()
            .path_with(Some(OsStr::new("/tmp/status-override")))
            .unwrap();
        assert!(access_path
            .ends_with(std::path::Path::new("status-override").join("access-status.json")));
    }

    #[test]
    fn empty_status_file_path_override_is_treated_as_unset() {
        // Standard env semantics: an empty value must not be honored — it
        // would make the path relative to the CWD. It resolves exactly like
        // an absent variable: the OS state dir + `iroh-tunnel` subpath.
        let empty = StatusWriter::serve()
            .path_with(Some(OsStr::new("")))
            .unwrap();
        let unset = StatusWriter::serve().path_with(None).unwrap();
        assert_eq!(empty, unset);
        assert_eq!(
            unset,
            crate::paths::default_state_dir()
                .unwrap()
                .join("serve-status.json")
        );
    }

    #[test]
    fn relative_status_file_path_override_is_absolutized() {
        // A relative override must resolve against the CWD once, at
        // resolution time — the returned path is absolute either way.
        let path = StatusWriter::serve()
            .path_with(Some(OsStr::new("rel/state")))
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(
            Path::new(&path).is_absolute(),
            "override path must be absolute, got {path}"
        );
    }

    #[tokio::test]
    async fn remove_own_status_file_spares_a_foreign_pid() {
        // Two processes sharing one state dir: this process's shutdown
        // cleanup must NOT remove a file another live instance wrote.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        StatusWriter::access()
            .save_to(
                &dir,
                &StatusPayload::Access(sample_access_status_with_pid(1)),
            )
            .unwrap();

        remove_own_status_file(StatusWriter::access(), Some(&dir)).await;

        assert!(
            dir.join("access-status.json").exists(),
            "a file owned by another pid must survive this process's cleanup"
        );
    }

    #[tokio::test]
    async fn remove_own_status_file_removes_own_pid() {
        // The file carries THIS process's pid — graceful shutdown owns it.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        StatusWriter::serve()
            .save_to(
                &dir,
                &StatusPayload::Serve(sample_status_with_pid(std::process::id())),
            )
            .unwrap();

        remove_own_status_file(StatusWriter::serve(), Some(&dir)).await;

        assert!(
            !dir.join("serve-status.json").exists(),
            "our own status file must be removed on cleanup"
        );
    }

    #[tokio::test]
    async fn remove_own_status_file_skips_missing_file() {
        // Never written (or already gone) — cleanup is a no-op, not an
        // error.
        let tmp = tempfile::tempdir().unwrap();
        remove_own_status_file(StatusWriter::serve(), Some(tmp.path())).await;
        assert!(!tmp.path().join("serve-status.json").exists());
    }
}
