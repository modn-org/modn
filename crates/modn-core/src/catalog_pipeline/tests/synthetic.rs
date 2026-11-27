//! Synthetic plans, previews and consents for digest and consent tests.
//!
//! Every value is built in memory. These tests read no manager profile and
//! touch no real Modn state.

use super::fixtures;
use crate::acquisition::ReleaseDigest;
use crate::approval::Capabilities;
use crate::buckets::BucketPackage;
use crate::catalog_pipeline::{
    plan_digest, CatalogPlan, Consent, ExecutableApproval, PlanKind, PlannedRelease, ReleasePreview,
};
use crate::planning::{InstallPlan, PlanStatus, PlannedFile};
use crate::transactions::{
    CatalogReference, CatalogRelease, DependencyState, NativeInjectorPolicy,
};
use std::path::PathBuf;

pub(super) fn reference(id: &str) -> CatalogReference {
    CatalogReference::new("main".to_owned(), "test-game".to_owned(), id.to_owned())
}

pub(super) fn pin(prefix: char) -> String {
    prefix.to_string().repeat(64)
}

pub(super) fn planned(id: &str, pin_prefix: char) -> PlannedRelease {
    let pin = pin(pin_prefix);
    let package = BucketPackage {
        bucket: "main".to_owned(),
        package: fixtures::manifest(id, Some(&pin)),
    };
    let mut release = CatalogRelease::new(reference(id), Vec::new());
    release.version = Some("1.0".to_owned());
    PlannedRelease {
        package,
        release,
        digest: ReleaseDigest::pinned(&pin).expect("pinned digest"),
        served: None,
        state: None,
        needed: Capabilities::default(),
        stored: None,
    }
}

pub(super) fn base_plan() -> CatalogPlan {
    CatalogPlan {
        kind: PlanKind::Install,
        selected: reference("top"),
        game_id: "test-game".to_owned(),
        installation: PathBuf::from("games/test-game"),
        order: vec![planned("top", 'a')],
        pending: Vec::new(),
        conflicts: Vec::new(),
        native_injector: NativeInjectorPolicy::Refuse,
        refusals: Vec::new(),
    }
}

pub(super) fn dependency_plan() -> CatalogPlan {
    let mut plan = base_plan();
    let mut dependency = planned("base", 'b');
    dependency.state = Some(DependencyState::Missing);
    plan.order = vec![dependency, planned("top", 'a')];
    plan
}

pub(super) fn install_plan(pin_prefix: char) -> InstallPlan {
    InstallPlan {
        archive_blake3: pin(pin_prefix),
        game_installation: PathBuf::from("games/test-game"),
        status: PlanStatus::Reviewable,
        metadata_only: true,
        operations: vec![PlannedFile {
            source: "pack/file.txt".to_owned(),
            destination: "addons/file.txt".to_owned(),
            bytes: 4,
            kind: "file-copy".to_owned(),
            owned_directory: None,
            root: "game".to_owned(),
        }],
        blockers: Vec::new(),
        skipped_generated_files: Vec::new(),
        content_base: None,
        content_root: None,
        overrides: Vec::new(),
    }
}

pub(super) fn preview(id: &str, pin_prefix: char) -> ReleasePreview {
    ReleasePreview {
        reference: reference(id),
        plan: install_plan(pin_prefix),
    }
}

pub(super) fn approval(id: &str, executables: &[&str]) -> ExecutableApproval {
    ExecutableApproval::new(
        reference(id),
        executables.iter().map(|path| (*path).to_owned()),
    )
}

pub(super) fn consent_for(
    plan: &CatalogPlan,
    native_injector: bool,
    approvals: Vec<ExecutableApproval>,
    previews: &[ReleasePreview],
) -> Consent {
    let digest = plan_digest(plan, &approvals, previews);
    Consent::new(native_injector, approvals, digest)
}
