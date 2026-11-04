use super::{InstallPlan, PlanStatus};
use std::path::Path;

/// Preview individual manual files selected by the current game module.
/// Existing targets are allowed only for adoption. Contents remain unverified.
/// No directory or unknown descendant becomes an ownership claim.
pub fn preview_adoption(game_id: &str, game: &Path, archive: &Path) -> anyhow::Result<InstallPlan> {
    let mut proposal = super::preview_install_zip(game_id, game, archive)?;
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
