//! Transactional activation of admitted game-module releases.
//!
//! Registration and catalog sync admit and stage every release first. These
//! functions then write one activation intent, move the version directories,
//! write the `current` pointers and remove the intent. The intent lets the next
//! open complete a set that a process death interrupted, or roll it back when
//! retained bytes do not match the recorded digest. An undo removes the intent
//! last, so a death during the undo also leaves the set repairable.

use super::super::loader::VerifiedModule;
use super::admission::AdmittedRelease;
use super::layout;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub(in crate::game_modules) const ACTIVATION: &str = "activation.json";
const SCHEMA_VERSION: u32 = 1;

/// Unit tests arm one registry root to fail its next `current` restore.
#[cfg(test)]
static FAIL_NEXT_POINTER_RESTORE: std::sync::Mutex<std::collections::BTreeSet<PathBuf>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());

/// Make the next undo of `root` fail one `current` restore.
#[cfg(test)]
pub(in crate::game_modules) fn fail_next_pointer_restore(root: &Path) {
    let mut armed = FAIL_NEXT_POINTER_RESTORE.lock().expect("seam lock");
    armed.insert(root.to_path_buf());
}

#[cfg(test)]
fn injected_pointer_restore_failure(root: &Path) -> bool {
    let mut armed = FAIL_NEXT_POINTER_RESTORE.lock().expect("seam lock");
    armed.remove(root)
}

/// One admitted release in an activation set.
pub(in crate::game_modules) struct ActivationRelease {
    pub(in crate::game_modules) game_id: String,
    pub(in crate::game_modules) version: String,
    pub(in crate::game_modules) blake3: String,
    pub(in crate::game_modules) previous: Option<String>,
    /// Staged directory relative to the registry root.
    pub(in crate::game_modules) staged: PathBuf,
    pub(in crate::game_modules) newer: bool,
}

/// Changes one activation made before it either finished or was undone.
#[derive(Debug, Default)]
pub(in crate::game_modules) struct ActivationProgress {
    moved: Vec<MovedRelease>,
    switched: Vec<(PathBuf, String)>,
    created: Vec<PathBuf>,
}

#[derive(Debug)]
struct MovedRelease {
    staged: PathBuf,
    target: PathBuf,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ActivationIntent {
    pub(super) schema_version: u32,
    pub(super) releases: Vec<IntentRelease>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct IntentRelease {
    pub(super) game_id: String,
    pub(super) version: String,
    pub(super) blake3: String,
    pub(super) previous: Option<String>,
    pub(super) staged: PathBuf,
}

/// Write one intent for the releases that this activation will make active.
///
/// The intent is written atomically before the first directory move.
pub(in crate::game_modules) fn write_intent(
    root: &Path,
    releases: &[ActivationRelease],
) -> Result<()> {
    let releases: Vec<IntentRelease> = releases
        .iter()
        .filter(|release| release.newer)
        .map(|release| IntentRelease {
            game_id: release.game_id.clone(),
            version: release.version.clone(),
            blake3: release.blake3.clone(),
            previous: release.previous.clone(),
            staged: release.staged.clone(),
        })
        .collect();
    if releases.is_empty() {
        return Ok(());
    }
    let intent = ActivationIntent {
        schema_version: SCHEMA_VERSION,
        releases,
    };
    let mut temp = tempfile::Builder::new()
        .prefix(".activation-")
        .tempfile_in(root)?;
    serde_json::to_writer(temp.as_file_mut(), &intent)?;
    temp.as_file().sync_all()?;
    temp.persist(root.join(ACTIVATION))
        .map_err(|error| error.error)?;
    Ok(())
}

/// Move every staged release into its version directory.
pub(in crate::game_modules) fn move_releases(
    root: &Path,
    releases: &[ActivationRelease],
    progress: &mut ActivationProgress,
) -> Result<()> {
    for release in releases {
        let directory = root.join(&release.game_id);
        if release.previous.is_none() {
            progress.created.push(directory.clone());
        }
        fs::create_dir_all(directory.join(layout::VERSIONS))?;
        let staged = root.join(&release.staged);
        let target = layout::release_directory(&directory, &release.version);
        fs::rename(&staged, &target).with_context(|| {
            format!(
                "cannot activate game module {} version {}",
                release.game_id, release.version
            )
        })?;
        progress.moved.push(MovedRelease { staged, target });
    }
    Ok(())
}

/// Write the `current` pointer of every release that sorts above its previous one.
pub(in crate::game_modules) fn write_current_pointers(
    root: &Path,
    releases: &[ActivationRelease],
    progress: &mut ActivationProgress,
) -> Result<()> {
    for release in releases {
        if !release.newer {
            continue;
        }
        let directory = root.join(&release.game_id);
        layout::write_current(&directory, &release.version).with_context(|| {
            format!(
                "cannot switch game module {} to version {}",
                release.game_id, release.version
            )
        })?;
        if let Some(previous) = &release.previous {
            progress.switched.push((directory, previous.clone()));
        }
    }
    Ok(())
}

/// Remove the intent after the last pointer is written.
pub(in crate::game_modules) fn finish_activation(root: &Path) -> Result<()> {
    remove_intent(root)
}

/// Activate one set, including the intent and pointer writes.
pub(in crate::game_modules) fn activate(
    root: &Path,
    releases: &[ActivationRelease],
    progress: &mut ActivationProgress,
) -> Result<()> {
    write_intent(root, releases)?;
    move_releases(root, releases, progress)?;
    write_current_pointers(root, releases, progress)?;
    finish_activation(root)
}

/// Activate the one release that a single registration admitted.
pub(in crate::game_modules) fn activate_admitted(
    root: &Path,
    manifest: &VerifiedModule,
    staged: &Path,
    game: &str,
    release: &AdmittedRelease,
) -> Result<()> {
    let relative = staged
        .strip_prefix(root)
        .with_context(|| format!("staged game module {game} is outside the registry"))?;
    let set = [ActivationRelease {
        game_id: game.to_owned(),
        version: manifest.version.clone(),
        blake3: release.blake3.clone(),
        previous: release.current.clone(),
        staged: relative.to_path_buf(),
        newer: release.newer,
    }];
    let mut progress = ActivationProgress::default();
    if let Err(error) = activate(root, &set, &mut progress) {
        return match undo_activation(root, &progress) {
            Ok(()) => Err(error.context(
                "game module activation failed; the module registry was restored",
            )),
            Err(restore_error) => Err(error.context(format!(
                "game module activation failed and the module registry could not be fully restored: {restore_error:#}"
            ))),
        };
    }
    Ok(())
}

/// Undo the changes recorded in `progress`, then remove the intent.
///
/// The intent stays on disk until every step succeeds. A death before the last
/// step therefore leaves a set that the next open repairs, even when only some
/// `current` pointers were restored.
pub(in crate::game_modules) fn undo_activation(
    root: &Path,
    progress: &ActivationProgress,
) -> Result<()> {
    let mut failures = Vec::new();
    for (directory, previous) in progress.switched.iter().rev() {
        if let Err(error) = restore_current(root, directory, previous) {
            failures.push(format!(
                "cannot restore the active release in {}: {error:#}",
                directory.display()
            ));
        }
    }
    for moved in progress.moved.iter().rev() {
        if let Err(error) =
            fs::rename(&moved.target, &moved.staged).or_else(|_| fs::remove_dir_all(&moved.target))
        {
            failures.push(format!(
                "cannot remove the partial release {}: {error:#}",
                moved.target.display()
            ));
        }
    }
    for directory in progress.created.iter().rev() {
        match fs::remove_dir_all(directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => failures.push(format!(
                "cannot remove the new module entry {}: {error:#}",
                directory.display()
            )),
        }
    }
    if failures.is_empty() {
        if let Err(error) = remove_intent(root) {
            failures.push(format!("cannot remove the activation intent: {error:#}"));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        bail!("{}", failures.join("; "))
    }
}

/// Restore one `current` pointer, subject to the test failure seam.
fn restore_current(root: &Path, directory: &Path, previous: &str) -> Result<()> {
    #[cfg(test)]
    if injected_pointer_restore_failure(root) {
        bail!("injected activation pointer restore failure");
    }
    #[cfg(not(test))]
    let _ = root;
    layout::write_current(directory, previous)
}

pub(super) fn remove_intent(root: &Path) -> Result<()> {
    match fs::remove_file(root.join(ACTIVATION)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("cannot remove the activation intent"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moved_release(root: &Path, game: &str, version: &str) -> MovedRelease {
        let staged = root.join("staging").join(game);
        fs::create_dir_all(&staged).expect("staging directory");
        fs::write(staged.join("module.json"), b"{}").expect("staged manifest");
        let game_directory = root.join("game-modules").join(game);
        fs::create_dir_all(game_directory.join(layout::VERSIONS)).expect("versions directory");
        let target = layout::release_directory(&game_directory, version);
        fs::rename(&staged, &target).expect("move staged release");
        MovedRelease { staged, target }
    }

    #[test]
    fn undo_removes_partial_moves_and_created_entries_and_the_intent() {
        let root = tempfile::tempdir().expect("root");
        let new_release = moved_release(root.path(), "new-game", "2.0.0");
        let new_directory = root.path().join("game-modules/new-game");
        let old_release = moved_release(root.path(), "old-game", "2.0.0");
        let old_directory = root.path().join("game-modules/old-game");
        layout::write_current(&old_directory, "2.0.0").expect("simulated switch");
        fs::write(root.path().join(ACTIVATION), b"{}").expect("intent");
        let old_target = old_release.target.clone();
        let old_staged = old_release.staged.clone();
        let progress = ActivationProgress {
            moved: vec![new_release, old_release],
            switched: vec![(old_directory.clone(), "1.0.0".to_owned())],
            created: vec![new_directory.clone()],
        };

        undo_activation(root.path(), &progress).expect("undo");

        assert!(fs::symlink_metadata(&new_directory).is_err());
        assert_eq!(
            layout::read_current(&old_directory).expect("restored current"),
            "1.0.0"
        );
        assert!(fs::symlink_metadata(&old_target).is_err());
        assert!(old_staged.join("module.json").is_file());
        assert!(!root.path().join(ACTIVATION).exists());
    }
}
