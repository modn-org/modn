//! Planning and committing catalog updates through the shared pipeline.

use super::commit_support::{digest, manifest, CommitFixture, FakeTransport, GAME};
use super::fixtures::tree;
use super::update_support::{commit, consent, install, plan, release, update_facts};
use crate::buckets::PackageSource;
use crate::catalog_pipeline::{plan_digest, Consent, DependentRequirement, PlanRefusal};
use crate::error::{code_of, ErrorCode};
use crate::progress::OperationControl;
use crate::transactions::{CatalogDependency, CatalogReference};
use std::fs;

#[test]
fn an_update_downloads_the_new_release_and_replaces_the_old_one() {
    let fixture = CommitFixture::new();
    let old = fixture.versioned_plugin("top-1.zip", "top", "v1");
    let new = fixture.versioned_plugin("top-2.zip", "top", "v2");
    fixture.add(&manifest("top", Some(&digest(&old))));
    let mut store = fixture.open_store();
    install(
        &fixture,
        &mut store,
        &old,
        "top",
        release("top", "1.0", Vec::new()),
    );
    drop(store);
    let mut target = manifest("top", Some(&digest(&new)));
    target.version = "2.0".to_owned();
    fixture.add(&target);
    let plan = {
        let state = fixture.readonly_store();
        plan(&fixture, &state, "top", &[])
    };
    assert_eq!(plan.pending.len(), 1);
    assert!(!update_facts(&plan).installed.id.is_empty());
    let consent = consent(&fixture, &plan, &new);
    let transport = FakeTransport::default();
    transport.respond(
        "https://example.test/top.zip",
        &fs::read(&new).expect("new bytes"),
    );
    let mut store = fixture.open_store();

    let outcome = commit(
        &fixture,
        &mut store,
        &plan,
        &consent,
        &transport,
        &OperationControl::default(),
    )
    .expect("update commit");

    assert_eq!(transport.calls(), ["https://example.test/top.zip"]);
    assert_eq!(outcome.attempts.len(), 1);
    assert!(!outcome.up_to_date);
    assert_eq!(outcome.packages.len(), 1);
    assert!(outcome.operation_id.is_some());
    assert_eq!(
        fs::read_to_string(fixture.game().join("addons/top.plugin")).expect("updated file"),
        "v2"
    );
    assert_eq!(fixture.count("managed_packages"), 1);
    assert_eq!(fixture.count("managed_files"), 1);
    for table in [
        "pending_transactions",
        "pending_files",
        "pending_links",
        "pending_stacks",
        "pending_stack_packages",
    ] {
        assert_eq!(fixture.count(table), 0, "{table}");
    }
    let history = fixture.history();
    assert_eq!(history.len(), 1);
    assert!(matches!(
        history[0].packages[0]
            .release_after
            .as_ref()
            .and_then(|release| release.source.as_ref()),
        Some(PackageSource::Https { url }) if url == "https://example.test/top.zip"
    ));
}

#[test]
fn an_up_to_date_plan_commits_nothing() {
    let fixture = CommitFixture::new();
    let old = fixture.versioned_plugin("top-1.zip", "top", "v1");
    fixture.add(&manifest("top", Some(&digest(&old))));
    let mut store = fixture.open_store();
    install(
        &fixture,
        &mut store,
        &old,
        "top",
        release("top", "1.0", Vec::new()),
    );
    drop(store);
    let plan = {
        let state = fixture.readonly_store();
        plan(&fixture, &state, "top", &[])
    };
    assert!(update_facts(&plan).up_to_date);
    assert!(plan.pending.is_empty());
    let consent = Consent::new(false, Vec::new(), plan_digest(&plan, &[], &[]));
    let before = tree(fixture.game());
    let mut store = fixture.open_store();
    let transport = FakeTransport::default();

    let outcome = commit(
        &fixture,
        &mut store,
        &plan,
        &consent,
        &transport,
        &OperationControl::default(),
    )
    .expect("up-to-date commit");

    assert!(outcome.up_to_date);
    assert!(outcome.packages.is_empty());
    assert!(outcome.operation_id.is_none());
    assert!(transport.calls().is_empty());
    assert_eq!(tree(fixture.game()), before);
    assert_eq!(fixture.count("managed_packages"), 1);
    assert_eq!(fixture.count("pending_transactions"), 0);
}

#[test]
fn a_pinned_package_is_refused() {
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
    store
        .set_package_pin(fixture.game(), &installed.id, true)
        .expect("pin");
    drop(store);
    let mut target = manifest("top", Some(&digest(&new)));
    target.version = "2.0".to_owned();
    fixture.add(&target);
    let plan = {
        let state = fixture.readonly_store();
        plan(&fixture, &state, "top", std::slice::from_ref(&new))
    };
    assert!(plan.refusals.iter().any(|refusal| matches!(
        refusal,
        PlanRefusal::PackagePinned { package_id } if package_id == &installed.id
    )));
    let consent = consent(&fixture, &plan, &new);
    let before = tree(fixture.game());
    let mut store = fixture.open_store();

    let error = commit(
        &fixture,
        &mut store,
        &plan,
        &consent,
        &FakeTransport::default(),
        &OperationControl::default(),
    )
    .expect_err("pinned package refused");

    assert_eq!(code_of(&error), Some(ErrorCode::PackagePinned));
    assert_eq!(tree(fixture.game()), before);
    assert_eq!(fixture.count("managed_packages"), 1);
    assert_eq!(fixture.count("pending_transactions"), 0);
}

#[test]
fn a_dependent_that_rejects_the_target_version_blocks_the_update() {
    let fixture = CommitFixture::new();
    let old = fixture.versioned_plugin("base-1.zip", "base", "v1");
    let new = fixture.versioned_plugin("base-2.zip", "base", "v2");
    let top = fixture.plugin_zip("top");
    fixture.add(&manifest("base", Some(&digest(&old))));
    fixture.add(&manifest("top", Some(&digest(&top))));
    let mut store = fixture.open_store();
    install(
        &fixture,
        &mut store,
        &old,
        "base",
        release("base", "1.0", Vec::new()),
    );
    let needs = CatalogDependency::new(
        CatalogReference::new("main".to_owned(), GAME.to_owned(), "base".to_owned()),
        digest(&old),
    )
    .with_requirement(Some("<2.0".to_owned()));
    install(
        &fixture,
        &mut store,
        &top,
        "top",
        release("top", "1.0", vec![needs]),
    );
    drop(store);
    let mut target = manifest("base", Some(&digest(&new)));
    target.version = "2.0".to_owned();
    fixture.add(&target);
    let plan = {
        let state = fixture.readonly_store();
        plan(&fixture, &state, "base", std::slice::from_ref(&new))
    };
    let update = update_facts(&plan);
    assert_eq!(update.dependents.len(), 1);
    assert_eq!(
        update.dependents[0].requirement,
        DependentRequirement::Requires("<2.0".to_owned())
    );
    assert!(!update.dependents[0].satisfied);
    assert!(plan.refusals.iter().any(|refusal| matches!(
        refusal,
        PlanRefusal::DependentBlocks { dependent } if !dependent.satisfied
    )));
    let consent = consent(&fixture, &plan, &new);
    let before = tree(fixture.game());
    let mut store = fixture.open_store();

    let error = commit(
        &fixture,
        &mut store,
        &plan,
        &consent,
        &FakeTransport::default(),
        &OperationControl::default(),
    )
    .expect_err("blocked by dependent");

    assert_eq!(code_of(&error), Some(ErrorCode::PlanBlocked));
    assert_eq!(tree(fixture.game()), before);
    assert_eq!(fixture.count("managed_packages"), 2);
    assert_eq!(fixture.count("pending_transactions"), 0);
}

#[test]
fn a_dependent_that_needs_the_archive_under_an_alias_blocks_the_update() {
    let fixture = CommitFixture::new();
    let old = fixture.versioned_plugin("base-1.zip", "base", "v1");
    let new = fixture.versioned_plugin("base-2.zip", "base", "v2");
    let top = fixture.plugin_zip("top");
    fixture.add(&manifest("base", Some(&digest(&old))));
    fixture.add(&manifest("top", Some(&digest(&top))));
    let mut store = fixture.open_store();
    install(
        &fixture,
        &mut store,
        &old,
        "base",
        release("base", "1.0", Vec::new()),
    );
    let alias = CatalogDependency::new(
        CatalogReference::new("other".to_owned(), GAME.to_owned(), "base".to_owned()),
        digest(&old),
    );
    install(
        &fixture,
        &mut store,
        &top,
        "top",
        release("top", "1.0", vec![alias]),
    );
    drop(store);
    let mut target = manifest("base", Some(&digest(&new)));
    target.version = "2.0".to_owned();
    fixture.add(&target);
    let plan = {
        let state = fixture.readonly_store();
        plan(&fixture, &state, "base", std::slice::from_ref(&new))
    };
    let update = update_facts(&plan);
    assert_eq!(update.dependents.len(), 1);
    assert_eq!(
        update.dependents[0].requirement,
        DependentRequirement::Alias("other/test-game/base".to_owned())
    );
    assert!(!update.dependents[0].satisfied);
    let consent = consent(&fixture, &plan, &new);
    let mut store = fixture.open_store();

    let error = commit(
        &fixture,
        &mut store,
        &plan,
        &consent,
        &FakeTransport::default(),
        &OperationControl::default(),
    )
    .expect_err("alias blocks the update");

    assert_eq!(code_of(&error), Some(ErrorCode::PlanBlocked));
    assert_eq!(fixture.count("managed_packages"), 2);
    assert_eq!(fixture.count("pending_transactions"), 0);
}

#[test]
fn an_uncovered_native_injector_is_refused() {
    let fixture = CommitFixture::new();
    let old = fixture.versioned_plugin("top-1.zip", "top", "v1");
    let new = fixture.versioned_plugin("top-2.zip", "top", "v2");
    fixture.add(&manifest("top", Some(&digest(&old))));
    let mut store = fixture.open_store();
    install(
        &fixture,
        &mut store,
        &old,
        "top",
        release("top", "1.0", Vec::new()),
    );
    drop(store);
    let mut target = manifest("top", Some(&digest(&new)));
    target.version = "2.0".to_owned();
    target.native_injector = true;
    fixture.add(&target);
    let plan = {
        let state = fixture.readonly_store();
        plan(&fixture, &state, "top", std::slice::from_ref(&new))
    };
    assert!(plan
        .refusals
        .iter()
        .any(|refusal| matches!(refusal, PlanRefusal::NativeInjector { .. })));
    let consent = consent(&fixture, &plan, &new);
    let before = tree(fixture.game());
    let mut store = fixture.open_store();

    let error = commit(
        &fixture,
        &mut store,
        &plan,
        &consent,
        &FakeTransport::default(),
        &OperationControl::default(),
    )
    .expect_err("native injector refused");

    assert_eq!(code_of(&error), Some(ErrorCode::NativeInjectorNotAllowed));
    assert_eq!(tree(fixture.game()), before);
    assert_eq!(fixture.count("managed_packages"), 1);
    assert_eq!(fixture.count("pending_transactions"), 0);
}
