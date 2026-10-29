//! Validate manager-state location without assuming any default game.
use anyhow::{bail, Context, Result};
use modn_core::discovery::validate_installation_for;
use std::path::{Path, PathBuf};

pub(super) fn database_path(state_dir: Option<&Path>) -> Result<PathBuf> {
    if let Some(dir) = state_dir {
        return Ok(dir.join("state.sqlite"));
    }
    if let Some(path) = std::env::var_os("LOCALAPPDATA") {
        return Ok(PathBuf::from(path).join("modn/state.sqlite"));
    }
    if let Some(path) = std::env::var_os("XDG_STATE_HOME") {
        return Ok(PathBuf::from(path).join("modn/state.sqlite"));
    }
    let home = std::env::var_os("HOME").context("no HOME or app state directory available")?;
    Ok(PathBuf::from(home).join(".local/state/modn/state.sqlite"))
}

pub(super) fn validate_state_path(
    state_path: &Path,
    selected_game: Option<&str>,
    selected_path: Option<&Path>,
    registered_roots: &[PathBuf],
) -> Result<()> {
    let selected_root = selected_path.map(Path::canonicalize).transpose()?;
    if let Ok(metadata) = std::fs::symlink_metadata(state_path) {
        if metadata.file_type().is_symlink() {
            bail!("state database must not be a filesystem link");
        }
    }
    let parent = state_path.parent().context("invalid state path")?;
    let mut directory = if parent.is_absolute() {
        parent.to_path_buf()
    } else {
        std::env::current_dir()?.join(parent)
    };
    loop {
        if let Ok(existing) = directory.canonicalize() {
            if existing.ancestors().any(|ancestor| {
                // Check the explicit module, never a baked-in default marker.
                selected_root
                    .as_ref()
                    .is_some_and(|root| existing.starts_with(root))
                    || registered_roots
                        .iter()
                        .any(|root| existing.starts_with(root))
                    || selected_game
                        .is_some_and(|game| validate_installation_for(game, ancestor).is_ok())
            }) {
                bail!("state directory must be outside a game installation");
            }
            return Ok(());
        }
        directory = directory
            .parent()
            .context("cannot resolve state directory")?
            .to_path_buf();
    }
}
