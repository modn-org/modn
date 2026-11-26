//! Canonical plan digests.

use super::synthetic::{approval, base_plan, dependency_plan, pin, preview, reference};
use crate::acquisition::ReleaseDigest;
use crate::buckets::PackageSource;
use crate::catalog_pipeline::{
    plan_digest, CatalogPlan, PendingDownload, PlanConflict, PlanDigest, PlanRefusal,
    ReleasePreview, ServedArchive, ServedOrigin,
};
use crate::planning::PlanStatus;
use crate::transactions::{DependencyState, NativeInjectorPolicy};
use std::path::PathBuf;

fn digest(plan: &CatalogPlan) -> PlanDigest {
    plan_digest(plan, &[], &[])
}

#[test]
fn same_inputs_give_the_same_digest() {
    let plan = base_plan();
    let previews = [preview("top", 'a')];
    let approvals = [approval("top", &["tools/helper.exe"])];

    let first = plan_digest(&plan, &approvals, &previews);
    let second = plan_digest(&plan, &approvals, &previews);

    assert_eq!(first, second);
    assert_eq!(first.as_str().len(), 64);
    assert_eq!(first.to_string(), first.as_str());
}

#[test]
fn preview_and_approval_order_do_not_change_the_digest() {
    let plan = dependency_plan();
    let top_approval = approval("top", &["tools/top.exe"]);
    let base_approval = approval("base", &["tools/base.exe"]);

    let forward = plan_digest(
        &plan,
        &[top_approval.clone(), base_approval.clone()],
        &[preview("top", 'a'), preview("base", 'b')],
    );
    let reordered = plan_digest(
        &plan,
        &[base_approval, top_approval],
        &[preview("base", 'b'), preview("top", 'a')],
    );

    assert_eq!(forward, reordered);
}

#[test]
fn dependency_id_order_does_not_change_the_digest() {
    let mut forward = dependency_plan();
    forward.order[0].state = Some(DependencyState::Ready {
        package_ids: vec!["main/test-game/a".to_owned(), "main/test-game/b".to_owned()],
    });
    let mut reversed = dependency_plan();
    reversed.order[0].state = Some(DependencyState::Ready {
        package_ids: vec!["main/test-game/b".to_owned(), "main/test-game/a".to_owned()],
    });

    assert_eq!(digest(&forward), digest(&reversed));
}

#[test]
fn the_order_changes_the_digest() {
    let mut changed = dependency_plan();
    changed.order.reverse();

    assert_ne!(digest(&dependency_plan()), digest(&changed));
}

#[test]
fn a_release_digest_changes_the_digest() {
    let mut changed = base_plan();
    changed.order[0].digest = ReleaseDigest::pinned(&pin('c')).expect("pinned digest");

    assert_ne!(digest(&base_plan()), digest(&changed));
}

#[test]
fn a_release_reference_changes_the_digest() {
    let mut changed = base_plan();
    changed.order[0].package.package.id = "renamed".to_owned();

    assert_ne!(digest(&base_plan()), digest(&changed));
}

#[test]
fn a_dependency_state_changes_the_digest() {
    let mut changed = dependency_plan();
    changed.order[0].state = Some(DependencyState::Ready {
        package_ids: Vec::new(),
    });

    assert_ne!(digest(&dependency_plan()), digest(&changed));
}

#[test]
fn a_conflict_changes_the_digest() {
    let mut changed = base_plan();
    changed.conflicts.push(PlanConflict {
        reference: reference("other"),
        installed: vec!["main/test-game/other".to_owned()],
    });

    assert_ne!(digest(&base_plan()), digest(&changed));
}

#[test]
fn the_native_injector_policy_changes_the_digest() {
    let mut changed = base_plan();
    changed.native_injector = NativeInjectorPolicy::Allow;

    assert_ne!(digest(&base_plan()), digest(&changed));
}

#[test]
fn approved_executables_change_the_digest() {
    let plan = base_plan();
    let approved = [approval("top", &["tools/helper.exe"])];

    assert_ne!(
        plan_digest(&plan, &[], &[]),
        plan_digest(&plan, &approved, &[])
    );
}

#[test]
fn equivalent_executable_paths_give_the_same_digest() {
    let plan = base_plan();
    let forward = [approval("top", &["tools/helper.exe"])];
    let backward = [approval("top", &[r"tools\helper.exe"])];

    assert_eq!(
        plan_digest(&plan, &forward, &[]),
        plan_digest(&plan, &backward, &[])
    );
}

#[test]
fn a_preview_changes_the_digest() {
    let plan = base_plan();
    let reviewable = [preview("top", 'a')];
    let mut blocked_plan = super::synthetic::install_plan('a');
    blocked_plan.status = PlanStatus::Blocked;
    let blocked = [ReleasePreview {
        reference: reference("top"),
        plan: blocked_plan,
    }];

    assert_ne!(
        plan_digest(&plan, &[], &reviewable),
        plan_digest(&plan, &[], &blocked)
    );
}

#[test]
fn the_game_id_changes_the_digest() {
    let mut changed = base_plan();
    changed.game_id = "other-game".to_owned();

    assert_ne!(digest(&base_plan()), digest(&changed));
}

#[test]
fn the_installation_path_changes_the_digest() {
    let mut changed = base_plan();
    changed.installation = PathBuf::from("games/other-game");

    assert_ne!(digest(&base_plan()), digest(&changed));
}

#[test]
fn path_separators_do_not_change_the_digest() {
    let mut changed = base_plan();
    changed.installation = PathBuf::from(r"games\test-game");

    assert_eq!(digest(&base_plan()), digest(&changed));
}

#[test]
fn a_served_archive_path_does_not_change_the_digest() {
    let mut changed = base_plan();
    changed.order[0].served = Some(ServedArchive {
        path: PathBuf::from("cache/top.zip"),
        source: PackageSource::Https {
            url: "https://example.test/top.zip".to_owned(),
        },
        digest: ReleaseDigest::pinned(&pin('a')).expect("pinned digest"),
        origin: ServedOrigin::Cache,
    });

    assert_eq!(digest(&base_plan()), digest(&changed));
}

#[test]
fn a_pending_download_becoming_served_does_not_change_the_digest() {
    let mut pending = base_plan();
    pending.pending.push(PendingDownload {
        reference: reference("top"),
        version: "1.0".to_owned(),
        digest: ReleaseDigest::pinned(&pin('a')).expect("pinned digest"),
        sources: Vec::new(),
        downloadable: true,
    });
    let mut served = base_plan();
    served.order[0].served = Some(ServedArchive {
        path: PathBuf::from("cache/top.zip"),
        source: PackageSource::Https {
            url: "https://example.test/top.zip".to_owned(),
        },
        digest: ReleaseDigest::pinned(&pin('a')).expect("pinned digest"),
        origin: ServedOrigin::Cache,
    });

    assert_eq!(digest(&pending), digest(&served));
}

#[test]
fn an_unmatched_archive_path_does_not_change_the_digest() {
    let mut changed = base_plan();
    changed.refusals.push(PlanRefusal::ArchiveUnmatched {
        path: PathBuf::from("downloads/unrelated.zip"),
    });

    assert_eq!(digest(&base_plan()), digest(&changed));
}
