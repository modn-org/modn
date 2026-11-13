//! Install-time native injector approval checks.
use crate::catalog_install::reference_of;
use crate::commands::InstallArgs;
use crate::dependency_plan;
use crate::install_consent;
use crate::selection::selected_installation;
use crate::state_access::ManagerState;
use anyhow::Result;
use modn_core::approval::{ApprovalRefusal, Uncovered};
use modn_core::buckets::BucketPackage;
use std::collections::BTreeMap;
use std::path::Path;

pub(super) fn check_native_approvals(
    selected: &BucketPackage,
    order: &[BucketPackage],
    args: &InstallArgs,
    state_path: &Path,
) -> Result<()> {
    if args.allow_native_injector || args.dry_run {
        return Ok(());
    }
    let state = ManagerState::load_deferred(state_path)?;
    let game = selected_installation(
        state.existing(),
        args.path.clone(),
        Some(&selected.package.game),
    )
    .ok()
    .map(|(_, installation)| installation);
    let installing: Vec<&BucketPackage> = match &game {
        Some(installation) => {
            let planned = dependency_plan::plan(
                order,
                &BTreeMap::new(),
                &BTreeMap::new(),
                state.existing(),
                installation,
            )?;
            order
                .iter()
                .zip(&planned)
                .filter(|(_, dependency)| dependency.missing())
                .map(|(release, _)| release)
                .chain([selected])
                .collect()
        }
        // Game selection can fail for ordinary reasons, such as no registration
        // or a missing --path. Refuse native injectors conservatively so the
        // approval error still precedes those generic errors.
        None => order.iter().chain([selected]).collect(),
    };
    for package in installing {
        if !package.package.native_injector {
            continue;
        }
        let reference = reference_of(package);
        let stored = match &game {
            Some(game) => install_consent::stored_approval(state.existing(), game, &reference)?,
            None => None,
        };
        if !stored.is_some_and(|approval| approval.native_injector) {
            return Err(ApprovalRefusal::new(reference, Uncovered::native_injector()).into_error());
        }
    }
    Ok(())
}
