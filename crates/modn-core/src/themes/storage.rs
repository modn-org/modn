//! Paths and the write lock shared by the theme registry and selection store.

use super::error::{Result, ThemeError};
use fs2::FileExt;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

/// Directory under the Modn data directory that holds every theme record.
pub(super) const THEMES: &str = "themes";
/// Lock file held exclusively for every theme write.
const LOCK: &str = "themes.lock";

/// Return the theme root for one Modn data directory.
pub(super) fn root(data_directory: &Path) -> PathBuf {
    data_directory.join(THEMES)
}

/// Create the theme root and take its exclusive lock for one write.
pub(super) fn lock(root: &Path) -> Result<File> {
    fs::create_dir_all(root)
        .map_err(|error| ThemeError::io("cannot create the theme data directory", error))?;
    ensure_real_directory(root, "theme data directory")?;
    let path = root.join(LOCK);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err(ThemeError::corrupt(
                &path,
                "the theme lock must be a regular file",
            ));
        }
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(ThemeError::io("cannot inspect the theme lock", error));
        }
        Ok(_) | Err(_) => {}
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|error| ThemeError::io("cannot open the theme lock", error))?;
    file.lock_exclusive()
        .map_err(|error| ThemeError::io("cannot lock the theme data directory", error))?;
    Ok(file)
}

/// Accept `path` only when it is a real directory, not a link.
pub(super) fn ensure_real_directory(path: &Path, what: &str) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => Ok(()),
        Ok(_) => Err(ThemeError::corrupt(
            path,
            format!("the {what} must be a real directory"),
        )),
        Err(error) => Err(ThemeError::io(format!("cannot inspect the {what}"), error)),
    }
}
