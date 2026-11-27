//! Shared helpers for the update pipeline tests.

use super::commit_support::{package, CommitFixture, FakeTransport, FetchResolver, GAME};
use super::fixtures;
use crate::acquisition::{CacheUse, SourcePolicy};
use crate::catalog_pipeline::{
    commit_plan, plan_digest, plan_update, preview_catalog_update, CatalogPlan, CommitOutcome,
    CommitRequest, Consent, ConsentInputs, InstallTarget, PlanKind, UpdateInput,
};
use crate::progress::OperationControl;
use crate::state::Store;
use crate::transactions::{
    CatalogDependency, CatalogReference, CatalogRelease, NativeInjectorPolicy,
};
use std::path::{Path, PathBuf};

pub(super) fn install(
    fixture: &CommitFixture,
    store: &mut Store,
    archive: &Path,
    _id: &str,
    release: CatalogRelease,
) -> crate::transactions::ManagedPackage {
    let proposal = fixtures::install_plan(fixture.game(), archive);
    store
        .install_catalog_release(
            fixture.game(),
            archive,
            &proposal,
            &release,
            NativeInjectorPolicy::Refuse,
        )
        .expect("catalog install")
}

pub(super) fn release(
    id: &str,
    version: &str,
    dependencies: Vec<CatalogDependency>,
) -> CatalogRelease {
    CatalogRelease::new(
        CatalogReference::new("main".to_owned(), GAME.to_owned(), id.to_owned()),
        dependencies,
    )
    .with_version(version.to_owned())
}

pub(super) fn plan(
    fixture: &CommitFixture,
    store: &Store,
    id: &str,
    supplied: &[PathBuf],
) -> CatalogPlan {
    let snapshot = fixture.snapshot();
    let selected = package(&snapshot, id);
    let cache = fixture.cache();
    plan_update(&UpdateInput::new(
        &selected,
        &snapshot,
        store,
        InstallTarget::new(GAME, fixture.game()),
        supplied,
        &cache,
        CacheUse::Reuse,
        ConsentInputs::default(),
    ))
    .expect("update plan")
}

pub(super) fn consent(fixture: &CommitFixture, plan: &CatalogPlan, archive: &Path) -> Consent {
    let preview =
        preview_catalog_update(None, fixture.roots(), plan, archive, &[]).expect("update preview");
    Consent::new(false, Vec::new(), plan_digest(plan, &[], &[preview]))
}

pub(super) fn commit(
    fixture: &CommitFixture,
    store: &mut Store,
    plan: &CatalogPlan,
    consent: &Consent,
    transport: &FakeTransport,
    control: &OperationControl,
) -> anyhow::Result<CommitOutcome> {
    commit_plan(CommitRequest::new(
        fixture.roots(),
        &fixture.cache(),
        plan,
        consent,
        transport,
        &FetchResolver,
        SourcePolicy::default(),
        CacheUse::Reuse,
        control,
        store,
    ))
}

pub(super) fn update_facts(plan: &CatalogPlan) -> &crate::catalog_pipeline::UpdatePlan {
    let PlanKind::Update(update) = &plan.kind else {
        panic!("an update plan");
    };
    update
}
