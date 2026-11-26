//! Run the game module's install preview for the releases of one plan.
//!
//! The preview uses the existing `planning` entry points and the module's
//! declared `content_root`, exactly as the install command does. It executes
//! the game module and reads the archives, but it writes nothing and never
//! prompts. Every preview call of the catalog pipeline lives here, so a future
//! module-selection change touches one file.

use super::consent::{installs, ExecutableApproval};
use super::digest::ReleasePreview;
use super::plan::CatalogPlan;
use crate::content_root::appdata_directory;
use crate::game_modules::content_root_spec;
use crate::manager_roots::ManagerRoots;
use crate::planning::{
    preview_install_zip_with_appdata_permissions, preview_install_zip_with_permissions, InstallPlan,
};
use crate::transactions::CatalogReference;
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Preview every release that `plan` would install.
///
/// `archives` names the verified archive of each release, keyed by its
/// `bucket/game/mod` reference. The result is aligned with the installing
/// releases of [`CatalogPlan::order`], dependencies first and the selected
/// release last. The game module's `plan(files)` and declared `content_root`
/// run through the existing `planning` functions.
///
/// # Errors
///
/// Returns the error of the game module, of an archive inspection, or of a
/// release that has no archive in `archives`.
pub fn preview_catalog_install(
    roots: &ManagerRoots,
    plan: &CatalogPlan,
    archives: &BTreeMap<String, PathBuf>,
    approvals: &[ExecutableApproval],
) -> Result<Vec<ReleasePreview>> {
    let mut previews = Vec::new();
    for planned in &plan.order {
        if !installs(planned) {
            continue;
        }
        let reference = planned.package.reference();
        let key = reference.to_string();
        let archive = archives
            .get(&key)
            .with_context(|| format!("{key}: the plan has no archive to preview"))?;
        let approved = approved_for(&reference, approvals);
        previews.push(ReleasePreview {
            reference,
            plan: preview_release(roots, &plan.game_id, &plan.installation, archive, &approved)?,
        });
    }
    Ok(previews)
}

/// Preview one release's archive with the selected game module.
pub(crate) fn preview_release(
    roots: &ManagerRoots,
    game_id: &str,
    installation: &Path,
    archive: &Path,
    approved: &[String],
) -> Result<InstallPlan> {
    match content_root_spec(roots, game_id)? {
        Some(_) => {
            let appdata = appdata_directory()?;
            preview_install_zip_with_appdata_permissions(
                roots,
                game_id,
                installation,
                archive,
                approved,
                &appdata,
            )
        }
        None => {
            preview_install_zip_with_permissions(roots, game_id, installation, archive, approved)
        }
    }
}

/// The executable paths that `approvals` grant for one release.
pub(crate) fn approved_for(
    reference: &CatalogReference,
    approvals: &[ExecutableApproval],
) -> Vec<String> {
    approvals
        .iter()
        .filter(|approval| approval.reference == *reference)
        .flat_map(|approval| approval.executables.iter().cloned())
        .collect()
}
