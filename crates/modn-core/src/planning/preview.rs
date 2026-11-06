use super::{resolve_with_permissions, InstallPlan, PlanIssue, PlanStatus};
use std::path::Path;

/// Produce a read-only ZIP installation preview through a selected game module.
/// A reviewable proposal is not an approval to write game files.
pub fn preview_install_zip(
    game_id: &str,
    game: &Path,
    archive: &Path,
) -> anyhow::Result<InstallPlan> {
    preview_install_zip_with_permissions(game_id, game, archive, &[])
}

/// Preview with explicit, trusted per-destination tool-executable permissions.
pub fn preview_install_zip_with_permissions(
    game_id: &str,
    game: &Path,
    archive: &Path,
    approved: &[String],
) -> anyhow::Result<InstallPlan> {
    preview_update_with_permissions(game_id, game, archive, approved, &[])
}

/// Preview a ZIP installation with an explicit appdata base for content roots.
pub fn preview_install_zip_with_appdata(
    game_id: &str,
    game: &Path,
    archive: &Path,
    appdata: &Path,
) -> anyhow::Result<InstallPlan> {
    preview_update_with_appdata(game_id, game, archive, &[], &[], appdata)
}

/// Preview `archive` as an update that replaces `replaced`: the game-relative
/// files and exclusive folders of the installed release. The game policy
/// plans as if those paths were already removed. Installation revalidates
/// the proposal against the real disk after it removes them.
pub fn preview_update_with_permissions(
    game_id: &str,
    game: &Path,
    archive: &Path,
    approved: &[String],
    replaced: &[String],
) -> anyhow::Result<InstallPlan> {
    preview_update_optional_appdata(game_id, game, archive, approved, replaced, None)
}

/// Preview an update with an explicit appdata base for content roots.
pub fn preview_update_with_appdata(
    game_id: &str,
    game: &Path,
    archive: &Path,
    approved: &[String],
    replaced: &[String],
    appdata: &Path,
) -> anyhow::Result<InstallPlan> {
    preview_update_optional_appdata(game_id, game, archive, approved, replaced, Some(appdata))
}

pub(crate) fn preview_install_zip_optional_appdata(
    game_id: &str,
    game: &Path,
    archive: &Path,
    approved: &[String],
    replaced: &[String],
    appdata: Option<&Path>,
) -> anyhow::Result<InstallPlan> {
    preview_update_optional_appdata(game_id, game, archive, approved, replaced, appdata)
}

fn preview_update_optional_appdata(
    game_id: &str,
    game: &Path,
    archive: &Path,
    approved: &[String],
    replaced: &[String],
    appdata: Option<&Path>,
) -> anyhow::Result<InstallPlan> {
    let inspection = crate::archive::inspect(archive)?;
    let mut proposal =
        resolve_with_permissions(game_id, &inspection, game, approved, replaced, appdata)?;
    if !inspection.format.is_installable() {
        proposal.status = PlanStatus::Blocked;
        proposal.blockers.push(PlanIssue {
            code: "NOT_INSTALLABLE".into(),
            message: "Only ZIP and 7z archives can be installed".into(),
        });
    }
    Ok(proposal)
}
