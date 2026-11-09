//! Prepare and perform a validated game launch.
//!
//! A game module declares launch targets as data. Rust chooses the store URI
//! when detection shows that the store installed the selected folder, and
//! otherwise runs a validated executable inside that folder. Lua never
//! receives a process API.

use crate::detection::DetectedInstallation;
use crate::game_modules::{self, LaunchTargets};
use crate::integrity::{self, FileRoots};
use crate::scan::IntegrityStatus;
use crate::state::{recorded_root, RootKind, Store};
use crate::stores::Storefront;
use anyhow::{bail, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(test)]
mod tests;

/// Validated action that starts one game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum LaunchTarget {
    /// Open a store URI through the operating system handler.
    StoreUri {
        /// Store whose URI is opened.
        store: Storefront,
        /// Validated launch URI.
        uri: String,
    },
    /// Start a game executable with the game folder as its working directory.
    Executable {
        /// Canonical executable inside the game folder.
        path: PathBuf,
        /// Canonical game folder.
        working_directory: PathBuf,
    },
}

/// What `modn play` will start, plus non-fatal warnings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct LaunchPlan {
    /// Game identity.
    pub game_id: String,
    /// Canonical selected installation.
    pub installation: PathBuf,
    /// Validated launch target.
    pub target: LaunchTarget,
    /// Non-fatal warnings, such as changed manager-owned files.
    pub warnings: Vec<String>,
}

/// Choose a validated launch target for one installation.
///
/// Refuses while any transaction awaits recovery. Managed-file drift produces
/// a warning and never a refusal.
pub fn plan_launch(
    store: Option<&Store>,
    game_id: &str,
    installation: &Path,
    detected: &[DetectedInstallation],
) -> Result<LaunchPlan> {
    let installation = installation.canonicalize().map_err(|error| {
        anyhow::anyhow!(
            "LAUNCH_INSTALLATION_INVALID: cannot resolve {}: {error}",
            installation.display()
        )
    })?;
    if let Some(store) = store {
        ensure_no_pending(store)?;
    }
    let targets = game_modules::launch_targets(game_id)?;
    let target = select_target(game_id, &targets, &installation, detected)?;
    Ok(LaunchPlan {
        game_id: game_id.to_owned(),
        installation: installation.clone(),
        target,
        warnings: drift_warnings(store, &installation),
    })
}

/// Run the selected target through the supplied launcher.
///
/// Tests pass a recorder here. The caller owns the operating-system launcher.
pub fn execute_launch<F>(plan: &LaunchPlan, launcher: F) -> Result<()>
where
    F: FnOnce(&LaunchTarget) -> Result<()>,
{
    launcher(&plan.target)
}

fn ensure_no_pending(store: &Store) -> Result<()> {
    let pending: bool = store.connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM pending_transactions)
             OR EXISTS(SELECT 1 FROM pending_stacks)",
        [],
        |row| row.get(0),
    )?;
    if pending {
        bail!(
            "LAUNCH_RECOVERY_PENDING: an unfinished transaction must be recovered first; run 'modn recover'"
        );
    }
    Ok(())
}

fn select_target(
    game_id: &str,
    targets: &LaunchTargets,
    installation: &Path,
    detected: &[DetectedInstallation],
) -> Result<LaunchTarget> {
    let detected_store = detected
        .iter()
        .find(|found| found.game_id == game_id && found.path == installation)
        .map(|found| found.store);
    if let Some(store) = detected_store {
        if let Some(uri) = targets.uri(store) {
            return Ok(LaunchTarget::StoreUri {
                store,
                uri: uri.to_owned(),
            });
        }
    }
    if let Some(relative) = targets.executable() {
        return Ok(LaunchTarget::Executable {
            path: resolve_executable(installation, relative)?,
            working_directory: installation.to_path_buf(),
        });
    }
    bail!(
        "LAUNCH_TARGET_UNAVAILABLE: game module {game_id} declares no launch target for this installation; declare a store URI or launch.executable in module.json"
    );
}

fn resolve_executable(installation: &Path, relative: &str) -> Result<PathBuf> {
    let candidate = installation.join(relative);
    let path = candidate.canonicalize().map_err(|error| {
        anyhow::anyhow!(
            "LAUNCH_TARGET_UNSAFE: cannot resolve executable {relative:?} inside {}: {error}",
            installation.display()
        )
    })?;
    if !path.starts_with(installation) {
        bail!(
            "LAUNCH_TARGET_UNSAFE: executable {relative:?} resolves outside {}",
            installation.display()
        );
    }
    let metadata = std::fs::metadata(&path).map_err(|error| {
        anyhow::anyhow!("LAUNCH_TARGET_UNSAFE: cannot inspect executable {relative:?}: {error}")
    })?;
    if !metadata.is_file() {
        bail!("LAUNCH_TARGET_UNSAFE: executable {relative:?} is not a regular file");
    }
    let is_exe = path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"));
    if !is_exe {
        bail!("LAUNCH_TARGET_UNSAFE: executable {relative:?} is not an .exe file");
    }
    Ok(path)
}

fn drift_warnings(store: Option<&Store>, installation: &Path) -> Vec<String> {
    let Some(store) = store else {
        return Vec::new();
    };
    match drift_warning(store, installation) {
        Ok(Some(warning)) => vec![warning],
        Ok(None) => Vec::new(),
        Err(error) => vec![format!(
            "LAUNCH_DRIFT_CHECK_FAILED: cannot check manager-owned files: {error:#}"
        )],
    }
}

fn drift_warning(store: &Store, installation: &Path) -> Result<Option<String>> {
    let packages = store.installed(installation)?;
    if packages.is_empty() {
        return Ok(None);
    }
    let content = recorded_root(&store.connection, installation, RootKind::Content)?;
    let content_root = content.as_deref().filter(|path| {
        std::fs::symlink_metadata(path)
            .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
    });
    let roots = FileRoots {
        game: installation,
        content: content_root,
        content_unavailable: IntegrityStatus::Missing,
    };
    let (files, _) = integrity::owned_file_checks(store, &roots, packages);
    let changed = files
        .iter()
        .filter(|file| !file.status.is_in_place())
        .count();
    if changed == 0 {
        return Ok(None);
    }
    Ok(Some(format!(
        "LAUNCH_MANAGED_FILES_CHANGED: {changed} manager-owned file(s) are missing, modified or unsafe; run 'modn doctor' to inspect."
    )))
}

/// Start a validated target with the operating system launcher.
pub fn system_launch(target: &LaunchTarget) -> Result<()> {
    match target {
        LaunchTarget::StoreUri { uri, .. } => open_uri(uri),
        LaunchTarget::Executable {
            path,
            working_directory,
        } => Command::new(path)
            .current_dir(working_directory)
            .spawn()
            .map(|_child| ())
            .map_err(|error| {
                anyhow::anyhow!(
                    "LAUNCH_START_FAILED: cannot start {}: {error}",
                    path.display()
                )
            }),
    }
}

#[cfg(windows)]
fn open_uri(uri: &str) -> Result<()> {
    // Explorer delegates the URI to the registered shell handler. Passing it
    // as one argument avoids cmd.exe metacharacter parsing in Epic URIs.
    Command::new("explorer")
        .arg(uri)
        .spawn()
        .map(|_child| ())
        .map_err(|error| anyhow::anyhow!("LAUNCH_START_FAILED: cannot open {uri}: {error}"))
}

#[cfg(not(windows))]
fn open_uri(uri: &str) -> Result<()> {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    Command::new(opener)
        .arg(uri)
        .spawn()
        .map(|_child| ())
        .map_err(|error| anyhow::anyhow!("LAUNCH_START_FAILED: cannot open {uri}: {error}"))
}
