use crate::catalog_install::reference_of;
use crate::commands::InstallArgs;
use crate::dependency_plan;
use crate::install_consent;
use crate::selection::selected_installation;
use crate::state_access::ManagerState;
use anyhow::Result;
use modn_core::approval::{ApprovalRefusal, Uncovered};
use modn_core::buckets::BucketPackage;
use modn_core::manager_roots::ManagerRoots;
use std::collections::BTreeMap;

pub(super) fn check_native_approvals(
    selected: &BucketPackage,
    order: &[BucketPackage],
    args: &InstallArgs,
    roots: &ManagerRoots,
) -> Result<()> {
    if args.allow_native_injector || args.dry_run {
        return Ok(());
    }
    let state = ManagerState::load_deferred(roots)?;
    let game = selected_installation(
        roots,
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
