use super::{InstallPlan, PlanStatus};
use std::path::Path;

/// Preview individual manual files selected by the current game module.
/// Existing targets are allowed only for adoption. Contents remain unverified.
/// No directory or unknown descendant becomes an ownership claim.
pub fn preview_adoption(game_id: &str, game: &Path, archive: &Path) -> anyhow::Result<InstallPlan> {
    preview_adoption_optional_appdata(game_id, game, archive, None)
}

/// Preview adoption with an explicit appdata base for content roots.
pub fn preview_adoption_with_appdata(
    game_id: &str,
    game: &Path,
    archive: &Path,
    appdata: &Path,
) -> anyhow::Result<InstallPlan> {
    preview_adoption_optional_appdata(game_id, game, archive, Some(appdata))
}

fn preview_adoption_optional_appdata(
    game_id: &str,
    game: &Path,
    archive: &Path,
    appdata: Option<&Path>,
) -> anyhow::Result<InstallPlan> {
    let mut proposal =
        super::preview_install_zip_optional_appdata(game_id, game, archive, &[], &[], appdata)?;
    proposal.blockers.retain(|issue| {
        !matches!(
            issue.code.as_str(),
            "EXISTING_FILE" | "REDMOD_DESTINATION_OCCUPIED"
        )
    });
    for operation in &mut proposal.operations {
        operation.owned_directory = None;
    }
    proposal.status = if proposal.blockers.is_empty() {
        PlanStatus::Reviewable
    } else {
        PlanStatus::Blocked
    };
    Ok(proposal)
}
