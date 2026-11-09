use anyhow::{Context, Result};
use std::cell::RefCell;
use std::path::{Path, PathBuf};

thread_local! {
    static DATA_DIRECTORY: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static PROFILE_DIRECTORY: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Run an operation with an explicit data directory, restoring the previous selection afterward.
pub fn with_data_directory<T>(directory: &Path, action: impl FnOnce() -> T) -> T {
    struct Restore(Option<PathBuf>);
    impl Drop for Restore {
        fn drop(&mut self) {
            DATA_DIRECTORY.with(|slot| slot.replace(self.0.take()));
        }
    }
    let previous = DATA_DIRECTORY.with(|slot| slot.replace(Some(directory.to_path_buf())));
    let _restore = Restore(previous);
    action()
}

/// Run an operation with an explicit manager profile directory.
///
/// The profile directory holds manager-owned state that `--state-dir` selects,
/// such as the game-module registry and bucket trust records. The shared data
/// directory still supplies the official `main` bucket. The previous selection
/// is restored afterward, including during unwinding.
pub fn with_profile_directory<T>(directory: &Path, action: impl FnOnce() -> T) -> T {
    struct Restore(Option<PathBuf>);
    impl Drop for Restore {
        fn drop(&mut self) {
            PROFILE_DIRECTORY.with(|slot| slot.replace(self.0.take()));
        }
    }
    let previous = PROFILE_DIRECTORY.with(|slot| slot.replace(Some(directory.to_path_buf())));
    let _restore = Restore(previous);
    action()
}

/// Resolve Modn's data directory using the existing state-directory environment overrides.
pub fn data_directory() -> Result<PathBuf> {
    if let Some(directory) = DATA_DIRECTORY.with(|slot| slot.borrow().clone()) {
        return Ok(directory);
    }
    if let Some(path) = std::env::var_os("LOCALAPPDATA") {
        return Ok(PathBuf::from(path).join("modn"));
    }
    if let Some(path) = std::env::var_os("XDG_STATE_HOME") {
        return Ok(PathBuf::from(path).join("modn"));
    }
    let home = std::env::var_os("HOME").context("no HOME or app state directory available")?;
    Ok(PathBuf::from(home).join(".local/state/modn"))
}

/// Resolve the manager profile directory selected by `--state-dir`.
///
/// Without an explicit profile, this is the ordinary data directory. The
/// official `main` bucket keeps using [`data_directory`] even when a profile
/// is selected, so every profile shares the installed copy.
pub fn profile_directory() -> Result<PathBuf> {
    if let Some(directory) = PROFILE_DIRECTORY.with(|slot| slot.borrow().clone()) {
        return Ok(directory);
    }
    data_directory()
}
