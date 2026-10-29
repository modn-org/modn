//! Cyberpunk legacy ZIP policy adapter; no game-specific decisions in the executor.
use crate::archive::inspect;
use crate::discovery::validate_installation;
use crate::planning::{plan, resolve, InstallPlan, PlanIssue};
use crate::state::Store;
use crate::CYBERPUNK_ID;
use anyhow::{bail, Result};
use std::path::Path;

/// Preview only operations the current legacy ZIP transaction engine can execute.
///
/// The Luau mapping is untrusted. Until the installer consumes the same
/// validated mapping in a later milestone, reject previews that disagree with
/// the current Rust executor instead of showing a misleading dry run.
pub fn preview_install_zip(game: &Path, archive: &Path) -> Result<InstallPlan> {
    let inspection = inspect(archive)?;
    let mut preview = resolve(CYBERPUNK_ID, &inspection, game)?;
    let actual = plan(&inspection, &preview.game_installation);
    let agrees = actual.operations.len() == preview.operations.len()
        && actual
            .operations
            .iter()
            .zip(&preview.operations)
            .all(|(a, b)| {
                a.source == b.source
                    && a.destination == b.destination
                    && a.bytes == b.bytes
                    && a.kind == b.kind
            });
    if inspection.format != "zip"
        || actual.status != "reviewable"
        || actual
            .operations
            .iter()
            .any(|file| file.kind != "legacy-archive")
        || !agrees
    {
        preview.blockers.push(PlanIssue {
            code: "NOT_INSTALLABLE".into(),
            message: "This archive or proposed mapping is not supported by the current installer"
                .into(),
        });
        preview.status = "blocked".into();
    }
    Ok(preview)
}

impl Store {
    /// Install an approved Cyberpunk legacy .archive ZIP using the generic executor.
    pub fn install_zip(&mut self, game: &Path, archive: &Path) -> Result<super::ManagedPackage> {
        self.install_inner(game, archive, None)
    }

    pub(super) fn install_inner(
        &mut self,
        game: &Path,
        archive: &Path,
        fail_after: Option<usize>,
    ) -> Result<super::ManagedPackage> {
        let game = validate_installation(game)?;
        let inspection = inspect(archive)?;
        if inspection.format != "zip" {
            bail!("installation supports ZIP packages only");
        }
        let proposal = plan(&inspection, &game);
        if proposal.status != "reviewable" {
            bail!("installation plan blocked: {:?}", proposal.blockers);
        }
        if proposal
            .operations
            .iter()
            .any(|file| file.kind != "legacy-archive")
        {
            bail!("installation currently supports legacy .archive files only");
        }
        self.install_operations_inner(&game, archive, &proposal, fail_after)
    }
}
