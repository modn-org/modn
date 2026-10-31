//! Validate manager-state location without assuming any default game.
use anyhow::{bail, Context, Result};
use modn_core::discovery::recognizes_installation_for;
use std::path::{Component, Path, PathBuf};

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
    // A missing component followed by '..' can escape ancestor-by-ancestor
    // confinement checks before create_dir_all materializes the path.
    if state_path
        .components()
        .any(|component| component == Component::ParentDir)
    {
        bail!("state directory must not contain parent-directory components");
    }
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
            if selected_root
                .as_ref()
                .is_some_and(|root| existing.starts_with(root))
                || registered_roots
                    .iter()
                    .any(|root| existing.starts_with(root))
            {
                bail!("state directory must be outside a game installation");
            }
            if let Some(game) = selected_game {
                for ancestor in existing.ancestors() {
                    if recognizes_installation_for(game, ancestor)? {
                        bail!("state directory must be outside a game installation");
                    }
                }
            }
            return Ok(());
        }
        directory = directory
            .parent()
            .context("cannot resolve state directory")?
            .to_path_buf();
    }
}
