//! Committing planned catalog installs through the one core routine.

use super::commit_support::{
    digest, manifest, reference_of, CommitFixture, FakeTransport, FetchResolver, GAME,
};
use super::fixtures::{self, tree};
use crate::acquisition::{CacheUse, SourcePolicy};
use crate::catalog_pipeline::commit::{clear_before_transaction_hook, set_before_transaction_hook};
use crate::catalog_pipeline::{
    commit_plan, plan_digest, CatalogPlan, CommitOutcome, CommitRequest, Consent, StalePlan,
};
use crate::error::{code_of, ErrorCode};
use crate::progress::{CancellationToken, OperationControl};
use crate::state::Store;
use crate::transactions::{CatalogReference, CatalogRelease, NativeInjectorPolicy};
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

fn commit(
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

fn pending_plan(fixture: &CommitFixture, id: &str) -> (CatalogPlan, PathBuf) {
    let archive = fixture.plugin_zip(id);
    let pin = digest(&archive);
    fixture.add(&manifest(id, Some(&pin)));
    let snapshot = fixture.snapshot();
    let plan = fixture.plan(&snapshot, id, None, &[]);
    (plan, archive)
}

fn reviewed_consent(
    fixture: &CommitFixture,
    plan: &CatalogPlan,
    archives: &[(&str, &PathBuf)],
) -> Consent {
    let local = fixture.local_archives(archives);
    let previews = fixture.preview(plan, &local);
    fixture.consent(plan, &previews)
}

#[test]
fn a_single_install_downloads_one_missing_release_and_commits_it() {
    let fixture = CommitFixture::new();
    let (plan, archive) = pending_plan(&fixture, "top");
    assert_eq!(plan.pending.len(), 1);
    assert!(plan.order[0].served.is_none());
    let consent = reviewed_consent(&fixture, &plan, &[("top", &archive)]);
    let transport = FakeTransport::default();
    transport.respond(
        "https://example.test/top.zip",
        &fs::read(&archive).expect("archive bytes"),
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
    .expect("commit");

    assert_eq!(transport.calls(), ["https://example.test/top.zip"]);
    assert_eq!(outcome.packages.len(), 1);
    assert_eq!(outcome.attempts.len(), 1);
    assert!(outcome.sources.contains_key(&reference_of("top")));
    assert!(outcome.operation_id.is_some());
    assert!(fixture.game().join("addons/top.plugin").is_file());
    assert_eq!(fixture.count("managed_packages"), 1);
    assert_eq!(fixture.count("managed_files"), 1);
    assert_eq!(fixture.count("pending_transactions"), 0);
    assert_eq!(fixture.count("pending_stacks"), 0);
    let history = fixture.history();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].packages.len(), 1);
    assert!(matches!(
        history[0].packages[0]
            .release_after
            .as_ref()
            .and_then(|release| release.source.as_ref()),
        Some(crate::buckets::PackageSource::Https { url })
            if url == "https://example.test/top.zip"
    ));
}

#[test]
fn a_stack_installs_a_missing_dependency_and_the_selected_release() {
    let fixture = CommitFixture::new();
    let base = fixture.plugin_zip("base");
    let top = fixture.plugin_zip("top");
    let mut top_manifest = manifest("top", Some(&digest(&top)));
    top_manifest.dependencies = vec!["base".to_owned()];
    fixture.add(&manifest("base", Some(&digest(&base))));
    fixture.add(&top_manifest);
    let snapshot = fixture.snapshot();
    let plan = fixture.plan(&snapshot, "top", None, &[]);
    assert_eq!(plan.pending.len(), 2);
    let consent = reviewed_consent(&fixture, &plan, &[("base", &base), ("top", &top)]);
    let transport = FakeTransport::default();
    transport.respond(
        "https://example.test/base.zip",
        &fs::read(&base).expect("base bytes"),
    );
    transport.respond(
        "https://example.test/top.zip",
        &fs::read(&top).expect("top bytes"),
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
    .expect("stack commit");

    assert_eq!(
        transport.calls(),
        [
            "https://example.test/base.zip",
            "https://example.test/top.zip"
        ]
    );
    assert_eq!(outcome.packages.len(), 2);
    assert_eq!(outcome.attempts.len(), 2);
    assert!(outcome.operation_id.is_some());
    assert!(fixture.game().join("addons/base.plugin").is_file());
    assert!(fixture.game().join("addons/top.plugin").is_file());
    assert_eq!(fixture.count("managed_packages"), 2);
    assert_eq!(fixture.count("managed_files"), 2);
    assert_eq!(fixture.count("pending_transactions"), 0);
    assert_eq!(fixture.count("pending_stacks"), 0);
    let history = fixture.history();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].packages.len(), 2);
    for package in &history[0].packages {
        assert!(matches!(
            package
                .release_after
                .as_ref()
                .and_then(|release| release.source.as_ref()),
            Some(crate::buckets::PackageSource::Https { .. })
        ));
    }
}

#[test]
fn a_declared_conflict_is_refused_without_a_write() {
    let fixture = CommitFixture::new();
    let enemy_archive = fixture.raw_zip("enemy");
    let top = fixture.plugin_zip("top");
    let mut top_manifest = manifest("top", Some(&digest(&top)));
    top_manifest.conflicts = vec!["enemy".to_owned()];
    fixture.add(&manifest("enemy", Some(&"b".repeat(64))));
    fixture.add(&top_manifest);
    let snapshot = fixture.snapshot();
    let mut store = fixture.open_store();
    let enemy_plan = fixtures::install_plan(fixture.game(), &enemy_archive);
    store
        .install_catalog_release(
            fixture.game(),
            &enemy_archive,
            &enemy_plan,
            &CatalogRelease::new(
                CatalogReference::new("main".to_owned(), GAME.to_owned(), "enemy".to_owned()),
                Vec::new(),
            ),
            NativeInjectorPolicy::Refuse,
        )
        .expect("installed conflict");
    let plan = {
        let state = fixture.readonly_store();
        fixture.plan(&snapshot, "top", Some(&state), std::slice::from_ref(&top))
    };
    assert!(plan.refusals.iter().any(|refusal| matches!(
        refusal,
        crate::catalog_pipeline::PlanRefusal::Conflict { .. }
    )));
    let consent = reviewed_consent(&fixture, &plan, &[("top", &top)]);
    let tree_before = tree(fixture.game());

    let error = commit(
        &fixture,
        &mut store,
        &plan,
        &consent,
        &FakeTransport::default(),
        &OperationControl::default(),
    )
    .expect_err("conflict refused");

    assert_eq!(code_of(&error), Some(ErrorCode::Conflict));
    assert_eq!(tree(fixture.game()), tree_before);
    assert_eq!(fixture.count("managed_packages"), 1);
    assert_eq!(fixture.count("managed_files"), 1);
    assert_eq!(fixture.count("pending_transactions"), 0);
    assert!(fixture.history().is_empty());
}

#[test]
fn a_changed_module_output_makes_the_plan_stale_and_writes_nothing() {
    let fixture = CommitFixture::new();
    let archive = fixture.plugin_zip("top");
    fixture.add(&manifest("top", Some(&digest(&archive))));
    let snapshot = fixture.snapshot();
    let plan = fixture.plan(&snapshot, "top", None, std::slice::from_ref(&archive));
    let consent = reviewed_consent(&fixture, &plan, &[("top", &archive)]);
    let script = fs::read_to_string(fixture.module().join("main.lua")).expect("module script");
    fs::write(
        fixture.module().join("main.lua"),
        script.replace("\"addons/\"", "\"stale/\""),
    )
    .expect("changed module");
    let mut store = fixture.open_store();
    let tree_before = tree(fixture.game());

    let error = commit(
        &fixture,
        &mut store,
        &plan,
        &consent,
        &FakeTransport::default(),
        &OperationControl::default(),
    )
    .expect_err("stale plan refused");

    assert_eq!(code_of(&error), Some(ErrorCode::PlanStale));
    let stale = error.downcast_ref::<StalePlan>().expect("typed refusal");
    assert_eq!(stale.code(), ErrorCode::PlanStale);
    assert_eq!(tree(fixture.game()), tree_before);
    assert_eq!(fixture.count("managed_packages"), 0);
    assert_eq!(fixture.count("managed_files"), 0);
    assert_eq!(fixture.count("pending_transactions"), 0);
    assert_eq!(fixture.count("pending_files"), 0);
    assert_eq!(fixture.count("pending_stacks"), 0);
    assert_eq!(fixture.count("pending_links"), 0);
    assert!(fixture.history().is_empty());
}

#[test]
fn a_native_injector_without_consent_is_refused() {
    let fixture = CommitFixture::new();
    let archive = fixture.plugin_zip("top");
    let mut top_manifest = manifest("top", Some(&digest(&archive)));
    top_manifest.native_injector = true;
    fixture.add(&top_manifest);
    let snapshot = fixture.snapshot();
    let plan = fixture.plan(&snapshot, "top", None, std::slice::from_ref(&archive));
    assert!(plan.refusals.iter().any(|refusal| matches!(
        refusal,
        crate::catalog_pipeline::PlanRefusal::NativeInjector { .. }
    )));
    let consent = reviewed_consent(&fixture, &plan, &[("top", &archive)]);
    let mut store = fixture.open_store();
    let tree_before = tree(fixture.game());

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
    assert_eq!(tree(fixture.game()), tree_before);
    assert_eq!(fixture.count("managed_packages"), 0);
    assert_eq!(fixture.count("pending_transactions"), 0);
}

#[test]
fn a_cancelled_download_writes_nothing() {
    let fixture = CommitFixture::new();
    let (plan, archive) = pending_plan(&fixture, "top");
    let consent = reviewed_consent(&fixture, &plan, &[("top", &archive)]);
    let transport = FakeTransport::default();
    let token = CancellationToken::new();
    token.cancel();
    let control = OperationControl::new(token, |_event| {});
    let mut store = fixture.open_store();
    let tree_before = tree(fixture.game());

    let error =
        commit(&fixture, &mut store, &plan, &consent, &transport, &control).expect_err("cancelled");

    assert_eq!(code_of(&error), Some(ErrorCode::Cancelled));
    assert!(transport.calls().is_empty());
    assert_eq!(tree(fixture.game()), tree_before);
    assert_eq!(fixture.count("managed_packages"), 0);
    assert_eq!(fixture.count("pending_transactions"), 0);
    assert!(fixture.history().is_empty());
}

#[test]
fn a_consent_for_another_plan_is_refused_without_a_write() {
    let fixture = CommitFixture::new();
    let (plan, archive) = pending_plan(&fixture, "top");
    let other = fixture.plugin_zip("other");
    fixture.add(&manifest("other", Some(&digest(&other))));
    let snapshot = fixture.snapshot();
    let other_plan = fixture.plan(&snapshot, "other", None, std::slice::from_ref(&other));
    // A consent is plain data: it is built from explicit grants and a digest of
    // another plan. There is no conversion from a `CatalogPlan` to a `Consent`.
    let other_previews =
        fixture.preview(&other_plan, &fixture.local_archives(&[("other", &other)]));
    let consent = Consent::new(
        false,
        Vec::new(),
        plan_digest(&other_plan, &[], &other_previews),
    );
    assert_ne!(plan_digest(&plan, &[], &other_previews), consent.plan);
    let supplied = std::slice::from_ref(&archive);
    let plan = fixture.plan(&snapshot, "top", None, supplied);
    let mut store = fixture.open_store();

    let error = commit(
        &fixture,
        &mut store,
        &plan,
        &consent,
        &FakeTransport::default(),
        &OperationControl::default(),
    )
    .expect_err("another plan's consent refused");

    assert_eq!(code_of(&error), Some(ErrorCode::PlanStale));
    assert_eq!(fixture.count("managed_packages"), 0);
    assert_eq!(fixture.count("pending_transactions"), 0);
}

/// Point the test module at `stale/` instead of `addons/`.
fn rewrite_module_output(module: &Path) {
    let script = fs::read_to_string(module.join("main.lua")).expect("module script");
    fs::write(
        module.join("main.lua"),
        script.replace("\"addons/\"", "\"stale/\""),
    )
    .expect("changed module");
}

#[test]
fn a_module_change_after_the_pre_lock_check_makes_the_plan_stale() {
    let fixture = CommitFixture::new();
    let archive = fixture.plugin_zip("top");
    fixture.add(&manifest("top", Some(&digest(&archive))));
    let snapshot = fixture.snapshot();
    let plan = fixture.plan(&snapshot, "top", None, std::slice::from_ref(&archive));
    let consent = reviewed_consent(&fixture, &plan, &[("top", &archive)]);
    let mut store = fixture.open_store();
    let tree_before = tree(fixture.game());
    let fired = Rc::new(Cell::new(false));
    let hook = Rc::clone(&fired);
    let module = fixture.module().to_path_buf();
    set_before_transaction_hook(move || {
        hook.set(true);
        rewrite_module_output(&module);
    });

    let error = commit(
        &fixture,
        &mut store,
        &plan,
        &consent,
        &FakeTransport::default(),
        &OperationControl::default(),
    )
    .expect_err("stale plan refused");
    clear_before_transaction_hook();

    assert!(fired.get(), "the hook ran after the pre-lock check");
    assert_eq!(code_of(&error), Some(ErrorCode::PlanStale));
    let stale = error.downcast_ref::<StalePlan>().expect("typed refusal");
    assert_eq!(stale.code(), ErrorCode::PlanStale);
    assert_eq!(tree(fixture.game()), tree_before);
    assert_eq!(fixture.count("managed_packages"), 0);
    assert_eq!(fixture.count("managed_files"), 0);
    assert_eq!(fixture.count("pending_transactions"), 0);
}

#[test]
fn a_module_change_after_the_pre_lock_check_makes_a_stack_stale() {
    let fixture = CommitFixture::new();
    let base = fixture.plugin_zip("base");
    let top = fixture.plugin_zip("top");
    let mut top_manifest = manifest("top", Some(&digest(&top)));
    top_manifest.dependencies = vec!["base".to_owned()];
    fixture.add(&manifest("base", Some(&digest(&base))));
    fixture.add(&top_manifest);
    let snapshot = fixture.snapshot();
    let plan = fixture.plan(&snapshot, "top", None, &[base.clone(), top.clone()]);
    let consent = reviewed_consent(&fixture, &plan, &[("base", &base), ("top", &top)]);
    let mut store = fixture.open_store();
    let tree_before = tree(fixture.game());
    let fired = Rc::new(Cell::new(false));
    let hook = Rc::clone(&fired);
    let module = fixture.module().to_path_buf();
    set_before_transaction_hook(move || {
        hook.set(true);
        rewrite_module_output(&module);
    });

    let error = commit(
        &fixture,
        &mut store,
        &plan,
        &consent,
        &FakeTransport::default(),
        &OperationControl::default(),
    )
    .expect_err("stale stack refused");
    clear_before_transaction_hook();

    assert!(fired.get(), "the hook ran after the pre-lock check");
    assert_eq!(code_of(&error), Some(ErrorCode::PlanStale));
    let stale = error.downcast_ref::<StalePlan>().expect("typed refusal");
    assert_eq!(stale.code(), ErrorCode::PlanStale);
    assert_eq!(tree(fixture.game()), tree_before);
    assert_eq!(fixture.count("managed_packages"), 0);
    assert_eq!(fixture.count("managed_files"), 0);
    assert_eq!(fixture.count("pending_transactions"), 0);
    assert_eq!(fixture.count("pending_stacks"), 0);
}
