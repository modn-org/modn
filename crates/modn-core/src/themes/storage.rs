use super::error::{Result, ThemeError};
use fs2::FileExt;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

pub(super) const THEMES: &str = "themes";
const LOCK: &str = "themes.lock";

pub(super) fn root(data_directory: &Path) -> PathBuf {
    data_directory.join(THEMES)
}

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
