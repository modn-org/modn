//! Paths and the recoverable replacement used by the theme registry.

use super::super::error::{Result, ThemeError};
use super::super::problem::{Rule, ThemeProblem};
use super::super::storage;
use super::record::{self, RegistrationRecord};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Directory under the theme root that holds registered themes.
pub(super) const REGISTRY: &str = "registry";
/// File beside one copied theme that records its identity and digest.
pub(super) const RECORD: &str = "registration.json";
/// Directory inside one registration that holds the copied theme.
pub(super) const THEME: &str = "theme";
/// Prefix of a temporary directory inside the registry.
pub(super) const STAGING_PREFIX: &str = ".staging-";
/// Directory inside the registry that holds a previous registration.
pub(super) const BACKUP: &str = ".backup";
/// Current shape of `registration.json`.
pub(super) const SCHEMA_VERSION: u32 = 1;
/// Largest number of bytes copied into staging before inspection.
pub(super) const MAX_COPY_BYTES: u64 = 16 * 1024 * 1024;
/// Largest number of directory entries copied into staging before inspection.
pub(super) const MAX_COPY_ENTRIES: usize = 2048;

/// Return `<theme root>/registry`.
pub(super) fn registry_root(root: &Path) -> PathBuf {
    root.join(REGISTRY)
}

/// Return `<registry>/<game>/<id>`.
pub(super) fn entry_root(registry: &Path, game: &str, id: &str) -> PathBuf {
    registry.join(game).join(id)
}

/// Return `<registry>/.backup/<game>/<id>`.
pub(super) fn backup_root(registry: &Path, game: &str, id: &str) -> PathBuf {
    registry.join(BACKUP).join(game).join(id)
}

/// Return the copied theme directory inside one registration.
pub(super) fn theme_dir(entry: &Path) -> PathBuf {
    entry.join(THEME)
}

/// One named step that a test can make fail.
///
/// Production code always passes `None`; the variants exist so tests can stop
/// a replacement at a precise point without a test-only Cargo feature.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "failpoints are constructed only by tests")
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Failpoint {
    /// Stop after the previous registration moved to its backup.
    AfterBackup,
    /// Stop after the new copy and its record are in place.
    BeforeBackupDelete,
    /// Fail when the new copy moves into place, restoring the backup.
    InstallFails,
    /// Fail when the new copy moves into place and when the restore runs.
    InstallAndRestoreFail,
}

#[cfg(test)]
impl Failpoint {
    /// Return the stored code of this failpoint.
    pub(super) const fn code(self) -> u8 {
        match self {
            Self::AfterBackup => 1,
            Self::BeforeBackupDelete => 2,
            Self::InstallFails => 3,
            Self::InstallAndRestoreFail => 4,
        }
    }

    /// Return the failpoint stored as `code`, if any.
    pub(super) const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::AfterBackup),
            2 => Some(Self::BeforeBackupDelete),
            3 => Some(Self::InstallFails),
            4 => Some(Self::InstallAndRestoreFail),
            _ => None,
        }
    }
}

/// Remove temporary staging directories that an earlier crash left behind.
///
/// Only `.staging-*` entries are removed; a `.backup` entry is never touched.
/// A failed removal of one entry becomes a diagnostic instead of an error.
pub(super) fn remove_stale_staging(registry: &Path) -> Result<Vec<ThemeProblem>> {
    let mut problems = Vec::new();
    let entries = match fs::read_dir(registry) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(problems),
        Err(error) => return Err(ThemeError::io("cannot list the theme registry", error)),
        Ok(entries) => entries,
    };
    for entry in entries {
        let entry =
            entry.map_err(|error| ThemeError::io("cannot list the theme registry", error))?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !name.starts_with(STAGING_PREFIX) {
            continue;
        }
        let path = entry.path();
        let result = if entry
            .file_type()
            .map_err(|error| ThemeError::io("cannot inspect a staging directory", error))?
            .is_dir()
        {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        if let Err(error) = result {
            problems.push(
                ThemeProblem::new(
                    Rule::ManifestInvalid,
                    format!("cannot remove the stale staging directory: {error}"),
                )
                .with_path(format!("{REGISTRY}/{name}")),
            );
        }
    }
    Ok(problems)
}

/// Move a staged registration into place, replacing any earlier copy.
///
/// The previous registration is moved to `backup` before the new copy moves
/// in. The record is written after the copy is in place. When step 4 or 5
/// fails, the backup is restored. When that restore fails too, the backup is
/// kept and the returned error names its path.
pub(super) fn install(
    staging: &Path,
    registry: &Path,
    target: &Path,
    backup: &Path,
    record: &RegistrationRecord,
    failpoint: Option<Failpoint>,
) -> Result<()> {
    let Some(parent) = target.parent() else {
        return Err(ThemeError::corrupt(
            target,
            "the theme registration has no parent directory",
        ));
    };
    fs::create_dir_all(parent)
        .map_err(|error| ThemeError::io("cannot create the theme registration directory", error))?;
    storage::ensure_real_directory(parent, "theme registration directory")?;
    let live = match fs::symlink_metadata(target) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => {
            return Err(ThemeError::io(
                "cannot inspect the existing theme registration",
                error,
            ));
        }
        Ok(metadata) if !metadata.file_type().is_dir() => {
            return Err(ThemeError::corrupt(
                target,
                "an existing theme registration must be a real directory",
            ));
        }
        Ok(_) => true,
    };
    let backup_exists = match fs::symlink_metadata(backup) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(ThemeError::io("cannot inspect the theme backup", error)),
        Ok(metadata) if !metadata.file_type().is_dir() => {
            return Err(ThemeError::corrupt(
                backup,
                "an existing theme backup must be a real directory",
            ));
        }
        Ok(_) => true,
    };
    let had_previous = live || backup_exists;
    let mut holding = None;
    if live {
        let Some(backup_parent) = backup.parent() else {
            return Err(ThemeError::corrupt(
                backup,
                "the theme backup has no parent directory",
            ));
        };
        fs::create_dir_all(backup_parent)
            .map_err(|error| ThemeError::io("cannot create the theme backup directory", error))?;
        storage::ensure_real_directory(backup_parent, "theme backup directory")?;
        if backup_exists {
            // Recovery kept a verified backup because the live copy failed
            // verification. Keep that backup as the restore point and move
            // the unusable live copy into disposable holding space.
            let temp = tempfile::Builder::new()
                .prefix(STAGING_PREFIX)
                .tempdir_in(registry)
                .map_err(|error| {
                    ThemeError::io("cannot create a theme holding directory", error)
                })?;
            fs::rename(target, temp.path().join("previous")).map_err(|error| {
                ThemeError::io("cannot move the previous theme registration aside", error)
            })?;
            holding = Some(temp);
        } else {
            fs::rename(target, backup).map_err(|error| {
                ThemeError::io("cannot move the previous theme registration aside", error)
            })?;
        }
    }
    if failpoint == Some(Failpoint::AfterBackup) {
        return Err(injected(
            "the registration stopped after the previous theme moved to its backup",
        ));
    }
    if matches!(
        failpoint,
        Some(Failpoint::InstallFails | Failpoint::InstallAndRestoreFail)
    ) {
        let original = injected("cannot move the registered theme into place");
        if failpoint == Some(Failpoint::InstallAndRestoreFail) {
            return Err(kept_backup(backup, &original));
        }
        return Err(rollback(had_previous, backup, target, original));
    }
    if let Err(error) = fs::rename(staging, target) {
        let original = ThemeError::io("cannot move the registered theme into place", error);
        return Err(rollback(had_previous, backup, target, original));
    }
    if let Err(error) = record::write(target, record) {
        return Err(rollback(had_previous, backup, target, error));
    }
    if failpoint == Some(Failpoint::BeforeBackupDelete) {
        return Err(injected(
            "the registration stopped before the backup was deleted",
        ));
    }
    if backup_exists {
        // The new registration is complete. A failed deletion is retried by
        // recovery on the next operation.
        let _ = fs::remove_dir_all(backup);
    }
    drop(holding);
    Ok(())
}

/// Restore the backup or keep it when the restore cannot run.
fn rollback(had_previous: bool, backup: &Path, target: &Path, original: ThemeError) -> ThemeError {
    if !had_previous {
        let _ = remove_path(target);
        return original;
    }
    match restore(backup, target) {
        Ok(()) => original,
        Err(error) => error,
    }
}

/// Put the previous registration back after a failed replacement.
fn restore(backup: &Path, target: &Path) -> Result<()> {
    if let Err(error) = remove_path(target) {
        return Err(kept_backup(backup, &error));
    }
    fs::rename(backup, target).map_err(|error| {
        kept_backup(
            backup,
            &ThemeError::io("cannot restore the previous theme registration", error),
        )
    })
}

/// Remove one path if it exists.
fn remove_path(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ThemeError::io(
            "cannot inspect the theme registration",
            error,
        )),
        Ok(metadata) if metadata.file_type().is_dir() => fs::remove_dir_all(path)
            .map_err(|error| ThemeError::io("cannot remove the theme registration", error)),
        Ok(_) => fs::remove_file(path)
            .map_err(|error| ThemeError::io("cannot remove the theme registration", error)),
    }
}

/// Build an error that names the backup that was kept.
fn kept_backup(backup: &Path, original: &ThemeError) -> ThemeError {
    ThemeError::corrupt(
        backup,
        format!(
            "cannot restore the previous theme registration ({original}); the previous copy is kept at {}",
            backup.display()
        ),
    )
}

/// Build the error a test failpoint returns.
fn injected(context: &str) -> ThemeError {
    ThemeError::io(context, io::Error::other("injected test failure"))
}
