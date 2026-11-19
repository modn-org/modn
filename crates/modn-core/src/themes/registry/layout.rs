//! Paths and the atomic replacement used by the theme registry.

use super::super::error::{Result, ThemeError};
use super::super::storage;
use std::fs;
use std::path::{Path, PathBuf};

/// Directory under the theme root that holds registered themes.
pub(super) const REGISTRY: &str = "registry";
/// File beside one copied theme that records its identity and digest.
pub(super) const RECORD: &str = "registration.json";
/// Directory inside one registration that holds the copied theme.
pub(super) const THEME: &str = "theme";
/// Prefix of a temporary directory inside the registry.
pub(super) const STAGING_PREFIX: &str = ".staging-";
/// Current shape of `registration.json`.
pub(super) const SCHEMA_VERSION: u32 = 1;

/// Return `<theme root>/registry`.
pub(super) fn registry_root(root: &Path) -> PathBuf {
    root.join(REGISTRY)
}

/// Return `<registry>/<game>/<id>`.
pub(super) fn entry_root(registry: &Path, game: &str, id: &str) -> PathBuf {
    registry.join(game).join(id)
}

/// Return the copied theme directory inside one registration.
pub(super) fn theme_dir(entry: &Path) -> PathBuf {
    entry.join(THEME)
}

/// Remove temporary directories that an earlier crash left behind.
pub(super) fn remove_stale_staging(registry: &Path) -> Result<()> {
    let entries = match fs::read_dir(registry) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
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
        result.map_err(|error| ThemeError::io("cannot remove a stale staging directory", error))?;
    }
    Ok(())
}

/// Move a staged registration into place, replacing any earlier copy.
///
/// The earlier copy is moved aside first. When the new copy cannot be moved in,
/// the earlier copy is restored before the error returns.
pub(super) fn install(staging: &Path, registry: &Path, target: &Path) -> Result<()> {
    let Some(parent) = target.parent() else {
        return Err(ThemeError::corrupt(
            target,
            "the theme registration has no parent directory",
        ));
    };
    fs::create_dir_all(parent)
        .map_err(|error| ThemeError::io("cannot create the theme registration directory", error))?;
    storage::ensure_real_directory(parent, "theme registration directory")?;
    match fs::symlink_metadata(target) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return fs::rename(staging, target).map_err(|error| {
                ThemeError::io("cannot move the registered theme into place", error)
            });
        }
        Ok(metadata) if !metadata.file_type().is_dir() => {
            return Err(ThemeError::corrupt(
                target,
                "an existing theme registration must be a real directory",
            ));
        }
        Ok(_) => {}
        Err(error) => {
            return Err(ThemeError::io(
                "cannot inspect the existing theme registration",
                error,
            ));
        }
    }
    let holding = tempfile::Builder::new()
        .prefix(STAGING_PREFIX)
        .tempdir_in(registry)
        .map_err(|error| ThemeError::io("cannot create a theme holding directory", error))?;
    let aside = holding.path().join("previous");
    fs::rename(target, &aside).map_err(|error| {
        ThemeError::io("cannot move the previous theme registration aside", error)
    })?;
    #[cfg(test)]
    if super::take_injected_swap_failure() {
        restore(&aside, target)?;
        return Err(ThemeError::io(
            "cannot move the registered theme into place",
            std::io::Error::other("injected test failure"),
        ));
    }
    match fs::rename(staging, target) {
        Ok(()) => Ok(()),
        Err(error) => {
            restore(&aside, target)?;
            Err(ThemeError::io(
                "cannot move the registered theme into place",
                error,
            ))
        }
    }
}

/// Put the previous registration back after a failed replacement.
fn restore(aside: &Path, target: &Path) -> Result<()> {
    fs::rename(aside, target).map_err(|error| {
        ThemeError::corrupt(
            target,
            format!("cannot restore the previous theme registration: {error}"),
        )
    })
}
