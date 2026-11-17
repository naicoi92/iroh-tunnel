//! `config edit`: a scratch copy for an editor, committed back only when the
//! live file is unchanged and the edit is valid.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;

use super::mutate::validate_role_doc;
use super::resolve_config_path::resolve_config_path;
use super::store::{
    ensure_parent, parse_doc, read_optional, resolve_target, strip_retired_fields, write_atomic,
    ConfigLock,
};
use super::types::{EditAttempt, EditOutcome, EditRejection};
use crate::role::Role;

/// Copy the config to a scratch file next to it for editing.
pub fn prepare_edit(role: Role, config: Option<&Path>) -> Result<EditAttempt> {
    let path = resolve_target(&resolve_config_path(role, config)?);
    let original = read_optional(&path)?;
    let original_text = original.clone().unwrap_or_default();

    ensure_parent(&path)?;
    let scratch = scratch_path(&path);
    write_atomic(&scratch, original_text.as_bytes())?;
    Ok(EditAttempt {
        role,
        path,
        scratch,
        original,
        keep_scratch: false,
    })
}

impl EditAttempt {
    /// The file the editor should modify.
    pub fn scratch_path(&self) -> &Path {
        &self.scratch
    }

    /// Apply the edited scratch copy.
    ///
    /// It is applied only if the live file is unchanged since the copy was
    /// taken and the edited document is valid; otherwise the live file is left
    /// as it was. Whenever the edit is not saved, its text is kept in
    /// `<config>.rejected` (owner-only) and named. The scratch copy is kept
    /// (owner-only on Unix) and named instead only when that sidecar cannot be
    /// written or the scratch copy itself cannot be read.
    ///
    /// # Errors
    ///
    /// Fails when the scratch copy cannot be read, the writer lock is not
    /// acquired in time, or the live file cannot be read or replaced; the
    /// error says where the edited text is.
    pub fn commit(mut self) -> Result<EditOutcome> {
        let edited = match std::fs::read_to_string(&self.scratch) {
            Ok(text) => text,
            Err(e) => return Err(self.unreadable_scratch(e)),
        };
        match self.apply(&edited) {
            Ok(outcome) => {
                self.keep_scratch = matches!(outcome, EditOutcome::Rejected { sidecar: None, .. });
                Ok(outcome)
            }
            Err(e) => Err(self.recover(e, &edited)),
        }
    }

    /// Keep the text of a failed commit in the sidecar, or in the scratch copy
    /// when the sidecar cannot be written, and say where in the error.
    fn recover(&mut self, error: anyhow::Error, edited: &str) -> anyhow::Error {
        let (sidecar, kept) = keep_rejected(&self.path, &self.scratch, edited);
        self.keep_scratch = sidecar.is_none();
        error.context(format!("your edit was not saved{kept}"))
    }

    /// The scratch copy could not be read: it is the only place the text may
    /// be, so keep it.
    fn unreadable_scratch(&mut self, error: std::io::Error) -> anyhow::Error {
        self.keep_scratch = true;
        anyhow::Error::new(error)
            .context(format!(
                "failed to read edited copy: {}",
                self.scratch.display()
            ))
            .context(format!(
                "your edit was not saved; {}",
                scratch_note(&self.scratch)
            ))
    }

    fn apply(&self, edited: &str) -> Result<EditOutcome> {
        let path = &self.path;

        if edited == self.original.as_deref().unwrap_or_default() {
            return Ok(EditOutcome::Unchanged);
        }

        let _lock = ConfigLock::acquire(path)?;
        if read_optional(path)? != self.original {
            let (sidecar, kept) = keep_rejected(path, &self.scratch, edited);
            return Ok(EditOutcome::Rejected {
                reason: EditRejection::Conflict,
                sidecar,
                message: format!(
                    "{} changed while it was being edited; your edit was not saved{kept}. \
                     Re-apply it to the current file",
                    path.display()
                ),
            });
        }
        let checked = parse_doc(edited, path).and_then(|mut doc| {
            strip_retired_fields(&mut doc);
            validate_role_doc(self.role, &doc.to_string(), path)?;
            Ok(doc)
        });
        match checked {
            Ok(doc) => {
                write_atomic(path, doc.to_string().as_bytes())?;
                Ok(EditOutcome::Saved { path: path.clone() })
            }
            Err(e) => {
                let (sidecar, kept) = keep_rejected(path, &self.scratch, edited);
                Ok(EditOutcome::Rejected {
                    reason: EditRejection::Invalid,
                    sidecar,
                    message: format!("{e:#}; the file was not changed{kept}"),
                })
            }
        }
    }
}

/// Distinguishes concurrent edit attempts on one config within this process.
static NEXT_ATTEMPT: AtomicU64 = AtomicU64::new(0);

/// Private copy of the config handed to the editor.
fn scratch_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    let attempt = NEXT_ATTEMPT.fetch_add(1, Ordering::Relaxed);
    name.push(format!(".edit-{}-{attempt}.toml", std::process::id()));
    path.with_file_name(name)
}

/// Reminder appended to every message that names a file kept for recovery.
const DELETE_REMINDER: &str = "delete it once recovered, it may contain secrets";

/// Keep the rejected edit in `<config>.rejected` and describe where, for the
/// error message. A failure to keep it is reported instead of hidden, naming
/// the scratch copy that then still holds the text.
fn keep_rejected(path: &Path, scratch: &Path, edited: &str) -> (Option<PathBuf>, String) {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".rejected");
    let sidecar = path.with_file_name(name);
    match write_atomic(&sidecar, edited.as_bytes()) {
        Ok(()) => {
            let kept = format!("; your text is in {}; {DELETE_REMINDER}", sidecar.display());
            (Some(sidecar), kept)
        }
        Err(e) => (
            None,
            format!(
                "; could not keep your text in {} ({e:#}); {}",
                sidecar.display(),
                scratch_note(scratch)
            ),
        ),
    }
}

/// Where the text is when only the scratch copy may hold it. A kept scratch
/// copy is restricted to its owner first, as an editor may have replaced it
/// with default permissions.
fn scratch_note(scratch: &Path) -> String {
    if !scratch.exists() {
        return format!("it is gone, {} no longer exists", scratch.display());
    }
    restrict_to_owner(scratch);
    format!("it is still in {}; {DELETE_REMINDER}", scratch.display())
}

/// Best effort: the error naming the file matters more than its mode.
#[cfg(unix)]
fn restrict_to_owner(file: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_to_owner(_file: &Path) {}

#[cfg(test)]
mod tests {
    use super::super::fixtures::VALID_SERVE;
    use super::*;

    #[test]
    fn edit_commit_rejects_concurrent_change_and_keeps_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.toml");
        std::fs::write(&path, "# original\n").unwrap();
        let attempt = prepare_edit(Role::Serve, Some(&path)).unwrap();
        std::fs::write(&path, "# concurrent\n").unwrap();
        std::fs::write(attempt.scratch_path(), VALID_SERVE).unwrap();
        match attempt.commit().unwrap() {
            EditOutcome::Rejected {
                reason: EditRejection::Conflict,
                sidecar: Some(p),
                message,
            } => {
                assert_eq!(std::fs::read_to_string(&p).unwrap(), VALID_SERVE);
                assert!(message.contains("changed while it was being edited"));
                assert!(message.contains(&p.display().to_string()));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# concurrent\n");
    }

    #[test]
    fn edit_commit_rejects_invalid_text_and_names_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.toml");
        std::fs::write(&path, "# original\n").unwrap();
        let attempt = prepare_edit(Role::Serve, Some(&path)).unwrap();
        std::fs::write(attempt.scratch_path(), "not = [valid").unwrap();
        match attempt.commit().unwrap() {
            EditOutcome::Rejected {
                reason: EditRejection::Invalid,
                sidecar: Some(p),
                message,
            } => {
                assert!(message.contains(&p.display().to_string()));
                assert!(message.contains("the file was not changed"));
                assert!(message.contains("delete it once recovered"), "{message}");
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# original\n");
    }

    #[test]
    fn rejected_edit_keeps_scratch_when_sidecar_cannot_be_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.toml");
        std::fs::write(&path, "# original\n").unwrap();
        // A directory in the sidecar's place makes keeping the text there fail.
        std::fs::create_dir(dir.path().join("serve.toml.rejected")).unwrap();
        let attempt = prepare_edit(Role::Serve, Some(&path)).unwrap();
        let scratch = attempt.scratch_path().to_path_buf();
        std::fs::write(&scratch, "not = [valid").unwrap();
        match attempt.commit().unwrap() {
            EditOutcome::Rejected {
                sidecar: None,
                message,
                ..
            } => {
                assert!(
                    message.contains(&scratch.display().to_string()),
                    "{message}"
                );
                assert!(message.contains("delete it once recovered"), "{message}");
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(std::fs::read_to_string(&scratch).unwrap(), "not = [valid");
    }

    #[test]
    fn commit_error_keeps_scratch_when_sidecar_cannot_be_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.toml");
        std::fs::write(&path, "# original\n").unwrap();
        let attempt = prepare_edit(Role::Serve, Some(&path)).unwrap();
        let scratch = attempt.scratch_path().to_path_buf();
        std::fs::write(&scratch, VALID_SERVE).unwrap();
        // A directory as the live file makes reading it under the lock fail,
        // and one in the sidecar's place makes keeping the text there fail.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        std::fs::create_dir(dir.path().join("serve.toml.rejected")).unwrap();
        let message = format!("{:#}", attempt.commit().unwrap_err());
        assert!(
            message.contains(&scratch.display().to_string()),
            "{message}"
        );
        assert!(
            message.contains("could not keep your text in")
                && message.contains("serve.toml.rejected"),
            "{message}"
        );
        assert!(message.contains("delete it once recovered"), "{message}");
        assert_eq!(std::fs::read_to_string(&scratch).unwrap(), VALID_SERVE);
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_scratch_is_kept_and_named() {
        use std::os::unix::fs::PermissionsExt;
        // SAFETY: geteuid(2) has no preconditions and cannot fail.
        if unsafe { libc::geteuid() } == 0 {
            // Root reads the file regardless of its mode.
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.toml");
        std::fs::write(&path, "# original\n").unwrap();
        let attempt = prepare_edit(Role::Serve, Some(&path)).unwrap();
        let scratch = attempt.scratch_path().to_path_buf();
        std::fs::set_permissions(&scratch, std::fs::Permissions::from_mode(0o000)).unwrap();
        let message = format!("{:#}", attempt.commit().unwrap_err());
        assert!(
            message.contains(&scratch.display().to_string()),
            "{message}"
        );
        assert!(scratch.exists());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# original\n");
    }

    #[test]
    fn removed_scratch_is_reported_gone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.toml");
        std::fs::write(&path, "# original\n").unwrap();
        let attempt = prepare_edit(Role::Serve, Some(&path)).unwrap();
        std::fs::remove_file(attempt.scratch_path()).unwrap();
        let message = format!("{:#}", attempt.commit().unwrap_err());
        assert!(!message.contains("still in"), "{message}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# original\n");
    }

    #[test]
    fn dropped_edit_attempt_removes_scratch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.toml");
        std::fs::write(&path, "# original\n").unwrap();
        let attempt = prepare_edit(Role::Serve, Some(&path)).unwrap();
        let scratch = attempt.scratch_path().to_path_buf();
        assert!(scratch.exists());
        drop(attempt);
        assert!(!scratch.exists());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# original\n");
    }

    #[test]
    fn edit_of_missing_config_conflicts_if_created_meanwhile() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.toml");
        let attempt = prepare_edit(Role::Serve, Some(&path)).unwrap();
        std::fs::write(&path, "# created\n").unwrap();
        std::fs::write(attempt.scratch_path(), VALID_SERVE).unwrap();
        assert!(matches!(
            attempt.commit().unwrap(),
            EditOutcome::Rejected {
                reason: EditRejection::Conflict,
                ..
            }
        ));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# created\n");
    }

    #[test]
    fn concurrent_edit_attempts_use_distinct_scratch_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.toml");
        std::fs::write(&path, "# original\n").unwrap();
        let first = prepare_edit(Role::Serve, Some(&path)).unwrap();
        let second = prepare_edit(Role::Serve, Some(&path)).unwrap();
        assert_ne!(first.scratch_path(), second.scratch_path());
        let second_text = format!("{VALID_SERVE}# second\n");
        std::fs::write(first.scratch_path(), VALID_SERVE).unwrap();
        std::fs::write(second.scratch_path(), &second_text).unwrap();

        // Dropping one attempt must not remove the other's scratch.
        let third = prepare_edit(Role::Serve, Some(&path)).unwrap();
        let third_scratch = third.scratch_path().to_path_buf();
        drop(third);
        assert!(!third_scratch.exists());
        assert!(first.scratch_path().exists() && second.scratch_path().exists());

        assert!(matches!(first.commit().unwrap(), EditOutcome::Saved { .. }));
        match second.commit().unwrap() {
            EditOutcome::Rejected {
                reason: EditRejection::Conflict,
                sidecar: Some(p),
                ..
            } => assert_eq!(std::fs::read_to_string(&p).unwrap(), second_text),
            other => panic!("unexpected {other:?}"),
        }
    }
}
