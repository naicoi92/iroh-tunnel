//! Which binary a root-run registration executes, and the managed root-owned
//! copy of the running executable used when the resolved one is not trusted.
//!
//! Invariant: the binary of a root-run registration is writable by root only.
//! [`choose`] decides between registering the resolved binary directly and a
//! managed copy; [`materialize`] writes that copy through verified directory
//! handles; [`release`] removes a copy no registration references any more.
//! The OS-touching functions exist on Linux and macOS only; other targets
//! always register the resolved binary.
#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "wired into install and the verbs by unix-system-binary-trust tasks 4 and 5"
    )
)]

use std::path::{Path, PathBuf};

use super::types::RunningExecutable;

/// The binary a root-run registration will execute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ServiceBinary {
    /// The resolved binary, trusted as it is.
    Direct(PathBuf),
    /// A managed root-owned copy of the running executable.
    Managed {
        /// The executable the copy is made from.
        source: RunningExecutable,
        /// Where the copy is published and what the registration runs.
        copy: PathBuf,
    },
}

impl ServiceBinary {
    /// The path the registration runs.
    pub(crate) fn path(&self) -> &Path {
        match self {
            Self::Direct(path) => path,
            Self::Managed { copy, .. } => copy,
        }
    }
}

/// The escape-free tail every managed copy path of `role` ends with, so a
/// registration that names the copy can be recognised.
pub(crate) fn managed_needle(role: &str) -> String {
    format!("iroh-tunnel/bin/{role}/iroh-tunnel")
}

/// The root-owned system base for managed copies on `os`
/// (`std::env::consts::OS`).
fn base_for(os: &str) -> &'static str {
    match os {
        "macos" => "/Library/Application Support",
        _ => "/var/lib",
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[cfg_attr(
    not(test),
    allow(
        unused_imports,
        reason = "wired into install and the verbs by unix-system-binary-trust tasks 4 and 5"
    )
)]
pub(crate) use unix::{choose, managed_copy_path, materialize, release};

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix {
    use std::fs::File;
    use std::path::{Path, PathBuf};

    use anyhow::{anyhow, Context, Result};

    use super::{base_for, managed_needle, ServiceBinary};
    use crate::service::trust::{self, TrustedOwners};
    use crate::service::types::{AppliedEffect, RunningExecutable};

    /// File name of every managed copy.
    const COPY_NAME: &str = "iroh-tunnel";
    /// Mode of the managed copy and of the directories created for it.
    const MODE: u32 = 0o755;

    /// Test-only root replacing the per-OS base; the fixed suffix is still
    /// appended so every managed path contains [`managed_needle`].
    #[cfg(feature = "test-utils")]
    const ENV_TEST_MANAGED_BIN_DIR: &str = "IROH_TUNNEL_TEST_MANAGED_BIN_DIR";

    #[cfg(feature = "test-utils")]
    fn base() -> PathBuf {
        std::env::var_os(ENV_TEST_MANAGED_BIN_DIR)
            .filter(|dir| !dir.is_empty())
            .map_or_else(
                || PathBuf::from(base_for(std::env::consts::OS)),
                PathBuf::from,
            )
    }

    #[cfg(not(feature = "test-utils"))]
    fn base() -> PathBuf {
        PathBuf::from(base_for(std::env::consts::OS))
    }

    /// Where the managed copy of `role`'s binary lives: one copy per role, so
    /// re-installing one role never changes what another role's service runs.
    pub(crate) fn managed_copy_path(role: &str) -> PathBuf {
        base().join(managed_needle(role))
    }

    /// Register the canonical `resolved` binary when `verify` accepts it,
    /// otherwise a managed `copy` of the `running` executable. Writes nothing.
    ///
    /// # Errors
    ///
    /// [`CliError::Config`](crate::error::CliError::Config) when `resolved`
    /// cannot be canonicalized.
    pub(crate) fn choose(
        resolved: &Path,
        running: &RunningExecutable,
        copy: PathBuf,
        verify: &dyn Fn(&Path) -> Result<()>,
    ) -> Result<ServiceBinary> {
        let canonical = trust::canonical(resolved)?;
        match verify(&canonical) {
            Ok(()) => Ok(ServiceBinary::Direct(canonical)),
            Err(why) => {
                tracing::debug!("registering a managed copy instead of {resolved:?}: {why:#}");
                Ok(ServiceBinary::Managed {
                    source: running.clone(),
                    copy,
                })
            }
        }
    }

    /// Publish the managed copy of a [`ServiceBinary::Managed`]: create its
    /// directories (`0755`) through verified handles, stream the running
    /// executable into a temp file, `fchmod` it to `0755`, rename it over the
    /// copy (a planted link is replaced, never followed) and verify the
    /// result. [`ServiceBinary::Direct`] writes nothing.
    ///
    /// # Errors
    ///
    /// [`CliError::Permission`](crate::error::CliError::Permission) naming an
    /// unsafe directory or copy, or the I/O error of reading the executable
    /// or writing the copy; an existing copy is then left untouched.
    pub(crate) fn materialize(
        binary: &ServiceBinary,
        owners: TrustedOwners,
    ) -> Result<Option<AppliedEffect>> {
        let ServiceBinary::Managed { source, copy } = binary else {
            return Ok(None);
        };
        let role_dir = role_dir(copy)?;
        trust::ensure_dir(role_dir, owners, MODE)?;
        let mut bytes = File::open(&source.read_from).with_context(|| {
            format!(
                "cannot read the running executable {}",
                source.display.display()
            )
        })?;
        let temp = format!(".iroh-tunnel.new.{}", std::process::id());
        trust::publish(role_dir, owners, COPY_NAME, &temp, &mut bytes, MODE)?;
        trust::verify_binary(copy, owners)?;
        Ok(Some(AppliedEffect::ManagedBinaryWritten(copy.clone())))
    }

    /// Remove the managed `copy` and its role directory (when then empty)
    /// unless `referenced()` says a registration still names it. Removal is
    /// handle-relative and never follows a link; an absent copy is `None`.
    ///
    /// # Errors
    ///
    /// [`CliError::Permission`](crate::error::CliError::Permission) when the
    /// role directory or one above it is unsafe, or the unlink error.
    pub(crate) fn release(
        copy: &Path,
        owners: TrustedOwners,
        referenced: impl FnOnce() -> bool,
    ) -> Result<Option<AppliedEffect>> {
        if referenced() {
            return Ok(None);
        }
        let removed = trust::remove_entry(role_dir(copy)?, owners, COPY_NAME)?;
        Ok(removed.then(|| AppliedEffect::ManagedBinaryRemoved(copy.to_path_buf())))
    }

    /// The role directory of a managed `copy` path.
    fn role_dir(copy: &Path) -> Result<&Path> {
        match (copy.parent(), copy.file_name()) {
            (Some(dir), Some(name)) if name == COPY_NAME => Ok(dir),
            _ => Err(anyhow!(
                "{} is not a managed copy path (…/{COPY_NAME})",
                copy.display()
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_base_per_os() {
        assert_eq!(base_for("linux"), "/var/lib");
        assert_eq!(base_for("macos"), "/Library/Application Support");
    }

    #[test]
    fn managed_needle_is_the_per_role_tail() {
        assert_eq!(managed_needle("serve"), "iroh-tunnel/bin/serve/iroh-tunnel");
        assert_eq!(
            managed_needle("access"),
            "iroh-tunnel/bin/access/iroh-tunnel"
        );
    }

    #[test]
    fn service_binary_path_is_what_the_registration_runs() {
        let direct = ServiceBinary::Direct("/usr/bin/iroh-tunnel".into());
        assert_eq!(direct.path(), Path::new("/usr/bin/iroh-tunnel"));
        let source = RunningExecutable {
            display: "/a".into(),
            read_from: "/b".into(),
        };
        let copy = PathBuf::from("/var/lib/iroh-tunnel/bin/serve/iroh-tunnel");
        let managed = ServiceBinary::Managed {
            source,
            copy: copy.clone(),
        };
        assert_eq!(managed.path(), copy);
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod unix_tests {
    use std::cell::RefCell;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    use super::*;
    use crate::error::CliError;
    use crate::service::trust::TrustedOwners;
    use crate::service::types::AppliedEffect;

    fn me() -> u32 {
        // SAFETY: geteuid(2) has no preconditions and cannot fail.
        unsafe { libc::geteuid() }
    }

    fn mine() -> TrustedOwners {
        TrustedOwners::RootAnd(me())
    }

    fn mode_of(path: &Path) -> u32 {
        std::fs::symlink_metadata(path).unwrap().mode() & 0o7777
    }

    fn set_mode(path: &Path, mode: u32) {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    /// A canonical, 0755 temp base for managed copies.
    fn base() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        set_mode(&root, 0o755);
        (tmp, root)
    }

    fn copy_in(base: &Path, role: &str) -> PathBuf {
        base.join(managed_needle(role))
    }

    /// A running executable whose readable bytes differ from its display path.
    fn running(dir: &Path, bytes: &[u8]) -> RunningExecutable {
        let display = dir.join("display-iroh-tunnel");
        std::fs::write(&display, b"display bytes, never copied").unwrap();
        let read_from = dir.join("read-from");
        std::fs::write(&read_from, bytes).unwrap();
        RunningExecutable { display, read_from }
    }

    fn managed(source: RunningExecutable, copy: PathBuf) -> ServiceBinary {
        ServiceBinary::Managed { source, copy }
    }

    #[cfg(not(feature = "test-utils"))]
    #[test]
    fn managed_copy_path_is_per_role_under_the_production_base() {
        let base = Path::new(base_for(std::env::consts::OS));
        for role in ["serve", "access"] {
            assert_eq!(managed_copy_path(role), base.join(managed_needle(role)));
        }
    }

    #[cfg(feature = "test-utils")]
    #[test]
    fn managed_copy_path_is_per_role_under_base() {
        const NAME: &str =
            "service::service_binary::unix_tests::managed_copy_path_is_per_role_under_base";
        crate::service::fixtures::in_isolated_home(NAME, |_| {
            std::env::set_var("IROH_TUNNEL_TEST_MANAGED_BIN_DIR", "/tmp/x");
            for role in ["serve", "access"] {
                let path = managed_copy_path(role);
                assert_eq!(path, Path::new("/tmp/x").join(managed_needle(role)));
                assert!(path.to_str().unwrap().ends_with(&managed_needle(role)));
            }
            std::env::set_var("IROH_TUNNEL_TEST_MANAGED_BIN_DIR", "");
            let base = Path::new(base_for(std::env::consts::OS));
            assert_eq!(
                managed_copy_path("serve"),
                base.join(managed_needle("serve"))
            );
        });
    }

    #[test]
    fn choose_direct_when_verified() {
        let (_tmp, root) = base();
        let resolved = root.join("iroh-tunnel");
        std::fs::write(&resolved, "x").unwrap();
        let run = running(&root, b"r");
        let chosen = choose(&resolved, &run, root.join("copy"), &|_| Ok(())).unwrap();
        assert_eq!(chosen, ServiceBinary::Direct(resolved));
    }

    #[test]
    fn choose_managed_when_refused() {
        let (_tmp, root) = base();
        let resolved = root.join("iroh-tunnel");
        std::fs::write(&resolved, "x").unwrap();
        let run = running(&root, b"r");
        let copy = copy_in(&root, "serve");
        let refuse = |_: &Path| Err(CliError::Permission("unsafe".into()).into());
        let chosen = choose(&resolved, &run, copy.clone(), &refuse).unwrap();
        assert_eq!(chosen, managed(run, copy));
    }

    #[test]
    fn choose_resolves_a_link_before_verifying() {
        let (_tmp, root) = base();
        let target = root.join("real-iroh-tunnel");
        std::fs::write(&target, "x").unwrap();
        let link = root.join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let seen = RefCell::new(Vec::new());
        let verify = |p: &Path| {
            seen.borrow_mut().push(p.to_path_buf());
            Ok(())
        };
        let run = running(&root, b"r");
        let chosen = choose(&link, &run, root.join("copy"), &verify).unwrap();
        assert_eq!(seen.into_inner(), vec![target.clone()]);
        assert_eq!(chosen, ServiceBinary::Direct(target));
    }

    #[test]
    fn materialize_direct_writes_nothing() {
        let (_tmp, root) = base();
        let direct = ServiceBinary::Direct(root.join("iroh-tunnel"));
        assert_eq!(materialize(&direct, mine()).unwrap(), None);
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    }

    #[test]
    fn materialize_writes_copy_0755_with_running_bytes() {
        let (_tmp, root) = base();
        let run = running(&root, b"running bytes");
        let copy = copy_in(&root, "serve");
        let effect = materialize(&managed(run, copy.clone()), mine()).unwrap();
        assert_eq!(
            effect,
            Some(AppliedEffect::ManagedBinaryWritten(copy.clone()))
        );
        assert_eq!(std::fs::read(&copy).unwrap(), b"running bytes");
        assert_eq!(mode_of(&copy), 0o755);
        for dir in copy.ancestors().skip(1).take(3) {
            assert_eq!(mode_of(dir), 0o755, "{}", dir.display());
        }
        let entries: Vec<_> = std::fs::read_dir(copy.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(entries, vec![std::ffi::OsString::from("iroh-tunnel")]);
    }

    #[test]
    fn materialize_refreshes_and_leaves_other_role_bytes() {
        let (_tmp, root) = base();
        let serve = copy_in(&root, "serve");
        let access = copy_in(&root, "access");
        materialize(
            &managed(running(&root, b"old serve"), serve.clone()),
            mine(),
        )
        .unwrap();
        materialize(&managed(running(&root, b"access"), access.clone()), mine()).unwrap();

        materialize(
            &managed(running(&root, b"new serve"), serve.clone()),
            mine(),
        )
        .unwrap();

        assert_eq!(std::fs::read(&serve).unwrap(), b"new serve");
        assert_eq!(std::fs::read(&access).unwrap(), b"access");
    }

    #[test]
    fn materialize_refuses_unsafe_role_dir() {
        let (_tmp, root) = base();
        let copy = copy_in(&root, "serve");
        let role_dir = copy.parent().unwrap();
        std::fs::create_dir_all(role_dir).unwrap();
        for dir in role_dir.ancestors().skip(1).take(2) {
            set_mode(dir, 0o755);
        }
        set_mode(role_dir, 0o777);

        let err = materialize(&managed(running(&root, b"r"), copy.clone()), mine())
            .expect_err("an unsafe role dir must be refused");

        let Some(CliError::Permission(msg)) = err.downcast_ref::<CliError>() else {
            panic!("expected Permission, got {err:#}");
        };
        assert!(msg.contains(role_dir.to_str().unwrap()), "{msg}");
        assert!(msg.contains("0777"), "{msg}");
        assert_eq!(mode_of(role_dir), 0o777);
        assert_eq!(std::fs::read_dir(role_dir).unwrap().count(), 0);
    }

    #[test]
    fn materialize_replaces_planted_link() {
        let (_tmp, root) = base();
        let copy = copy_in(&root, "serve");
        std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
        for dir in copy.ancestors().skip(1).take(3) {
            set_mode(dir, 0o755);
        }
        let target = root.join("victim");
        std::fs::write(&target, b"victim bytes").unwrap();
        std::os::unix::fs::symlink(&target, &copy).unwrap();

        materialize(&managed(running(&root, b"running"), copy.clone()), mine()).unwrap();

        assert!(std::fs::symlink_metadata(&copy).unwrap().is_file());
        assert_eq!(std::fs::read(&copy).unwrap(), b"running");
        assert_eq!(std::fs::read(&target).unwrap(), b"victim bytes");
    }

    #[test]
    fn release_removes_copy_and_empty_dir_when_unreferenced() {
        let (_tmp, root) = base();
        let copy = copy_in(&root, "serve");
        materialize(&managed(running(&root, b"r"), copy.clone()), mine()).unwrap();

        let effect = release(&copy, mine(), || false).unwrap();

        assert_eq!(
            effect,
            Some(AppliedEffect::ManagedBinaryRemoved(copy.clone()))
        );
        let role_dir = copy.parent().unwrap();
        assert!(std::fs::symlink_metadata(role_dir).is_err());
        assert!(role_dir.parent().unwrap().is_dir());
    }

    #[test]
    fn release_keeps_copy_when_referenced() {
        let (_tmp, root) = base();
        let copy = copy_in(&root, "serve");
        materialize(&managed(running(&root, b"r"), copy.clone()), mine()).unwrap();

        assert_eq!(release(&copy, mine(), || true).unwrap(), None);
        assert_eq!(std::fs::read(&copy).unwrap(), b"r");
    }

    #[test]
    fn release_is_idempotent_when_absent() {
        let (_tmp, root) = base();
        let copy = copy_in(&root, "serve");
        assert_eq!(release(&copy, mine(), || false).unwrap(), None);
        materialize(&managed(running(&root, b"r"), copy.clone()), mine()).unwrap();
        release(&copy, mine(), || false).unwrap();
        assert_eq!(release(&copy, mine(), || false).unwrap(), None);
    }
}
