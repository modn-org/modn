//! The stored approval commits in the transaction of its ownership change (#173).

use super::commit_support::{digest, manifest, package, CommitFixture, FakeTransport, GAME};
use super::fixtures::tree;
use super::update_support::{commit, install, release};
use crate::acquisition::CacheUse;
use crate::approval::Capabilities;
use crate::catalog_pipeline::{
    plan_digest, plan_install, plan_update, preview_catalog_install, preview_catalog_update,
    CatalogPlan, Consent, ConsentInputs, InstallInput, InstallTarget, UpdateInput,
};
use crate::progress::OperationControl;
use crate::state::Store;
use std::path::PathBuf;

fn native_flags() -> ConsentInputs {
    ConsentInputs::new(Capabilities::new(true, Vec::<String>::new()))
}

fn install_plan(fixture: &CommitFixture, id: &str, supplied: &[PathBuf]) -> CatalogPlan {
    let snapshot = fixture.snapshot();
    let selected = package(&snapshot, id);
    let cache = fixture.cache();
    plan_install(&InstallInput::new(
        &selected,
        &snapshot,
        None,
        InstallTarget::new(GAME, fixture.game()),
        supplied,
        &cache,
        CacheUse::Reuse,
        native_flags(),
    ))
    .expect("install plan")
}

fn install_consent(
    fixture: &CommitFixture,
    plan: &CatalogPlan,
    archives: &[(&str, &PathBuf)],
) -> Consent {
    let local = fixture.local_archives(archives);
    let previews =
        preview_catalog_install(None, fixture.roots(), plan, &local, &[]).expect("preview");
    Consent::new(true, Vec::new(), plan_digest(plan, &[], &previews))
}

fn update_plan(fixture: &CommitFixture, store: &Store, id: &str, new: &PathBuf) -> CatalogPlan {
    let snapshot = fixture.snapshot();
    let selected = package(&snapshot, id);
    let cache = fixture.cache();
    plan_update(&UpdateInput::new(
        &selected,
        &snapshot,
        store,
        InstallTarget::new(GAME, fixture.game()),
        std::slice::from_ref(new),
        &cache,
        CacheUse::Reuse,
        native_flags(),
    ))
    .expect("update plan")
}

fn fail_approval_writes(fixture: &CommitFixture) {
    rusqlite::Connection::open(fixture.state_path())
        .expect("state")
        .execute_batch(
            "CREATE TRIGGER fail_approval BEFORE INSERT ON mod_approvals
             BEGIN SELECT RAISE(ABORT, 'injected approval failure'); END;",
        )
        .expect("trigger");
}

fn stored(store: &Store, fixture: &CommitFixture, id: &str) -> Option<Capabilities> {
    store
        .approval(fixture.game(), "main", id)
        .expect("approval")
}

fn native_single(fixture: &CommitFixture) -> (CatalogPlan, Consent) {
    let archive = fixture.plugin_zip("top");
    let mut top = manifest("top", Some(&digest(&archive)));
    top.native_injector = true;
    fixture.add(&top);
    let plan = install_plan(fixture, "top", std::slice::from_ref(&archive));
    assert!(plan.refusals.is_empty(), "{:?}", plan.refusals);
    let consent = install_consent(fixture, &plan, &[("top", &archive)]);
    (plan, consent)
}

#[test]
fn an_install_stores_its_approval_with_the_ownership_change() {
    let fixture = CommitFixture::new();
    let (plan, consent) = native_single(&fixture);
    let mut store = fixture.open_store();

    commit(
        &fixture,
        &mut store,
        &plan,
        &consent,
        &FakeTransport::default(),
        &OperationControl::default(),
    )
    .expect("install");

    assert_eq!(fixture.count("managed_packages"), 1);
    assert!(stored(&store, &fixture, "top").is_some_and(|approval| approval.native_injector));
}

#[test]
fn a_failed_approval_write_rolls_back_the_install() {
    let fixture = CommitFixture::new();
    let (plan, consent) = native_single(&fixture);
    let mut store = fixture.open_store();
    fail_approval_writes(&fixture);
    let before = tree(fixture.game());

    commit(
        &fixture,
        &mut store,
        &plan,
        &consent,
        &FakeTransport::default(),
        &OperationControl::default(),
    )
    .expect_err("approval write fails");
    store.recover().expect("recover");

    assert_eq!(tree(fixture.game()), before);
    assert_eq!(fixture.count("managed_packages"), 0);
    assert_eq!(fixture.count("pending_transactions"), 0);
    assert!(stored(&store, &fixture, "top").is_none());
}

#[test]
fn a_failed_approval_write_rolls_back_the_update() {
    let fixture = CommitFixture::new();
    let old = fixture.versioned_plugin("top-1.zip", "top", "v1");
    let new = fixture.versioned_plugin("top-2.zip", "top", "v2");
    fixture.add(&manifest("top", Some(&digest(&old))));
    let mut store = fixture.open_store();
    let installed = install(
        &fixture,
        &mut store,
        &old,
        "top",
        release("top", "1.0", Vec::new()),
    );
    let mut target = manifest("top", Some(&digest(&new)));
    target.version = "2.0".to_owned();
    target.native_injector = true;
    fixture.add(&target);
    let plan = update_plan(&fixture, &store, "top", &new);
    assert!(plan.refusals.is_empty(), "{:?}", plan.refusals);
    let preview =
        preview_catalog_update(None, fixture.roots(), &plan, &new, &[]).expect("update preview");
    let consent = Consent::new(true, Vec::new(), plan_digest(&plan, &[], &[preview]));
    fail_approval_writes(&fixture);
    let before = tree(fixture.game());

    commit(
        &fixture,
        &mut store,
        &plan,
        &consent,
        &FakeTransport::default(),
        &OperationControl::default(),
    )
    .expect_err("approval write fails");
    store.recover().expect("recover");

    assert_eq!(tree(fixture.game()), before);
    let packages = store.installed(fixture.game()).expect("installed");
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].id, installed.id);
    assert!(stored(&store, &fixture, "top").is_none());
}

#[test]
fn a_stack_stores_its_approvals_only_when_the_stack_commits() {
    let fixture = CommitFixture::new();
    let base = fixture.plugin_zip("base");
    let top = fixture.plugin_zip("top");
    let mut top_manifest = manifest("top", Some(&digest(&top)));
    top_manifest.dependencies = vec!["base".to_owned()];
    top_manifest.native_injector = true;
    fixture.add(&manifest("base", Some(&digest(&base))));
    fixture.add(&top_manifest);
    let plan = install_plan(&fixture, "top", &[base.clone(), top.clone()]);
    assert!(plan.refusals.is_empty(), "{:?}", plan.refusals);
    let consent = install_consent(&fixture, &plan, &[("base", &base), ("top", &top)]);
    let mut store = fixture.open_store();
    fail_approval_writes(&fixture);
    let before = tree(fixture.game());

    commit(
        &fixture,
        &mut store,
        &plan,
        &consent,
        &FakeTransport::default(),
        &OperationControl::default(),
    )
    .expect_err("approval write fails");
    store.recover().expect("recover");

    assert_eq!(tree(fixture.game()), before);
    assert_eq!(fixture.count("managed_packages"), 0);
    assert_eq!(fixture.count("pending_stacks"), 0);
    assert!(stored(&store, &fixture, "top").is_none());

    rusqlite::Connection::open(fixture.state_path())
        .expect("state")
        .execute_batch("DROP TRIGGER fail_approval;")
        .expect("drop trigger");
    commit(
        &fixture,
        &mut store,
        &plan,
        &consent,
        &FakeTransport::default(),
        &OperationControl::default(),
    )
    .expect("stack install");
    assert_eq!(fixture.count("managed_packages"), 2);
    assert!(stored(&store, &fixture, "top").is_some_and(|approval| approval.native_injector));
}
