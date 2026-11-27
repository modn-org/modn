//! Run the game module's install preview for the releases of one plan.
//!
//! The preview uses the existing `planning` entry points and the module's
//! declared `content_root`, exactly as the install command does. It executes
//! the game module and reads the archives, but it writes nothing and never
//! prompts. Every preview call of the catalog pipeline lives here, so a future
//! module-selection change touches one file.

use super::consent::ExecutableApproval;
use super::digest::ReleasePreview;
use super::plan::{CatalogPlan, PlanKind};
use crate::content_root::appdata_directory;
use crate::game_modules::content_root_spec;
use crate::manager_roots::ManagerRoots;
use crate::planning::{
    preview_install_zip_with_appdata_permissions, preview_install_zip_with_permissions,
    preview_update_with_appdata, preview_update_with_permissions, InstallPlan,
};
use crate::transactions::CatalogReference;
use anyhow::{bail, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Preview every member of `plan` that has an archive.
///
/// `archives` names the verified archive of each member, keyed by its
/// `bucket/game/mod` reference. [`CatalogPlan::members`] names the releases
/// this installation would place, dependencies first and the selected release
/// last. A member without an archive is skipped; the caller refuses it
/// separately. The game module's `plan(files)` and declared `content_root` run
/// through the existing `planning` functions.
///
/// # Errors
///
/// Returns the refusal of a shared archive that cannot be merged, the error of
/// the game module, or the error of an archive inspection.
pub fn preview_catalog_install(
    roots: &ManagerRoots,
    plan: &CatalogPlan,
    archives: &BTreeMap<String, PathBuf>,
    approvals: &[ExecutableApproval],
) -> Result<Vec<ReleasePreview>> {
    let mut previews = Vec::new();
    for planned in plan.members()? {
        let reference = planned.package.reference();
        let key = reference.to_string();
        let Some(archive) = archives.get(&key) else {
            continue;
        };
        let approved = approved_for(&reference, approvals);
        previews.push(ReleasePreview {
            reference,
            plan: preview_release(roots, &plan.game_id, &plan.installation, archive, &approved)?,
        });
    }
    Ok(previews)
}

/// Preview one release of `plan` from its archive.
///
/// The caller names the release and its verified archive, so this reads no
/// plan members and refuses nothing about the rest of the plan. A dry run uses
/// it to preview the selected release before any other archive is available.
///
/// # Errors
///
/// Returns the error of the game module or of the archive inspection.
pub fn preview_catalog_release(
    roots: &ManagerRoots,
    plan: &CatalogPlan,
    reference: &CatalogReference,
    archive: &Path,
    approved: &[String],
) -> Result<ReleasePreview> {
    Ok(ReleasePreview {
        reference: reference.clone(),
        plan: preview_release(roots, &plan.game_id, &plan.installation, archive, approved)?,
    })
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

/// Preview one update's archive with the paths of the installed release removed.
///
/// The game module plans as if `UpdatePlan::replaced` were already gone, so an
/// exclusive folder is claimed again. Installation revalidates the proposal
/// against the real disk after it removes them.
///
/// # Errors
///
/// Returns an error for a plan that is not an update, the error of the game
/// module, or the error of the archive inspection.
pub fn preview_catalog_update(
    roots: &ManagerRoots,
    plan: &CatalogPlan,
    archive: &Path,
    approved: &[String],
) -> Result<ReleasePreview> {
    let PlanKind::Update(update) = &plan.kind else {
        bail!("the plan is not an update");
    };
    Ok(ReleasePreview {
        reference: plan.selected.clone(),
        plan: preview_update(
            roots,
            &plan.game_id,
            &plan.installation,
            archive,
            approved,
            &update.replaced,
        )?,
    })
}

/// Preview one archive as an update that replaces `replaced`.
pub(crate) fn preview_update(
    roots: &ManagerRoots,
    game_id: &str,
    installation: &Path,
    archive: &Path,
    approved: &[String],
    replaced: &[String],
) -> Result<InstallPlan> {
    match content_root_spec(roots, game_id)? {
        Some(_) => {
            let appdata = appdata_directory()?;
            preview_update_with_appdata(
                roots,
                game_id,
                installation,
                archive,
                approved,
                replaced,
                &appdata,
            )
        }
        None => preview_update_with_permissions(
            roots,
            game_id,
            installation,
            archive,
            approved,
            replaced,
        ),
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
