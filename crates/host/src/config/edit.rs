//! `config edit`: a scratch copy for an editor, committed back only when the
//! live file is unchanged and the edit is valid.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

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
    /// as it was and the edited text is kept in `<config>.rejected`
    /// (owner-only) so it is not lost.
    pub fn commit(self) -> Result<EditOutcome> {
        let edited = std::fs::read_to_string(&self.scratch)
            .with_context(|| format!("failed to read edited copy: {}", self.scratch.display()));
        let _ = std::fs::remove_file(&self.scratch);
        let edited = edited?;
        let path = &self.path;

        if edited == self.original.as_deref().unwrap_or_default() {
            return Ok(EditOutcome::Unchanged);
        }

        let _lock = ConfigLock::acquire(path)?;
        if read_optional(path)? != self.original {
            let (sidecar, kept) = keep_rejected(path, &edited);
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
        let checked = parse_doc(&edited, path).and_then(|mut doc| {
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
                let (sidecar, kept) = keep_rejected(path, &edited);
                Ok(EditOutcome::Rejected {
                    reason: EditRejection::Invalid,
                    sidecar,
                    message: format!("{e:#}; the file was not changed{kept}"),
                })
            }
        }
    }
}

/// Private copy of the config handed to the editor.
fn scratch_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".edit-{}.toml", std::process::id()));
    path.with_file_name(name)
}

/// Keep the rejected edit in `<config>.rejected` and describe where, for the
/// error message. A failure to keep it is reported instead of hidden.
fn keep_rejected(path: &Path, edited: &str) -> (Option<PathBuf>, String) {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".rejected");
    let sidecar = path.with_file_name(name);
    match write_atomic(&sidecar, edited.as_bytes()) {
        Ok(()) => {
            let kept = format!("; your text is in {}", sidecar.display());
            (Some(sidecar), kept)
        }
        Err(e) => (None, format!("; could not keep your text ({e:#})")),
    }
}

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
            }
            other => panic!("unexpected {other:?}"),
        }
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
}
