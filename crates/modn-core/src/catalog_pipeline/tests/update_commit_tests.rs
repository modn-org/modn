//! Update commits through the shared pipeline: stale plans and refusals.

use super::commit_support::{digest, manifest, CommitFixture, FakeTransport, GAME};
use super::fixtures::tree;
use super::update_support::{commit, consent, install, plan, release, update_facts};
use crate::buckets::PackageSource;
use crate::error::{code_of, ErrorCode};
use crate::history::HistoryContext;
use crate::progress::{CancellationToken, OperationControl};
use crate::transactions::CatalogReference;

#[test]
fn a_changed_installed_package_makes_the_plan_stale_and_writes_nothing() {
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
    drop(store);
    let mut target = manifest("top", Some(&digest(&new)));
    target.version = "2.0".to_owned();
    fixture.add(&target);
    let plan = {
        let state = fixture.readonly_store();
        plan(&fixture, &state, "top", std::slice::from_ref(&new))
    };
    assert!(plan.refusals.is_empty(), "{:?}", plan.refusals);
    let consent = consent(&fixture, &plan, &new);
    let connection = rusqlite::Connection::open(fixture.state_path()).expect("state");
    connection
        .execute(
            "UPDATE managed_files SET relative_path='addons/moved.plugin' WHERE package_id=?1",
            [&installed.id],
        )
        .expect("changed installed state");
    drop(connection);
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
    .expect_err("stale update refused");

    assert_eq!(code_of(&error), Some(ErrorCode::PlanStale), "{error:#}");
    assert_eq!(tree(fixture.game()), before);
    assert_eq!(fixture.count("managed_packages"), 1);
    assert_eq!(fixture.count("managed_files"), 1);
    for table in [
        "pending_transactions",
        "pending_files",
        "pending_links",
        "pending_stacks",
    ] {
        assert_eq!(fixture.count(table), 0, "{table}");
    }
}

#[test]
fn a_cancelled_update_download_writes_nothing() {
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
    let consent = consent(&fixture, &plan, &new);
    let token = CancellationToken::new();
    token.cancel();
    let control = OperationControl::new(token, |_event| {});
    let before = tree(fixture.game());
    let transport = FakeTransport::default();
    let mut store = fixture.open_store();

    let error = commit(&fixture, &mut store, &plan, &consent, &transport, &control)
        .expect_err("cancelled download");

    assert_eq!(code_of(&error), Some(ErrorCode::Cancelled));
    assert!(transport.calls().is_empty());
    assert_eq!(tree(fixture.game()), before);
    assert_eq!(fixture.count("managed_packages"), 1);
    assert_eq!(fixture.count("pending_transactions"), 0);
}

#[test]
fn a_git_release_installed_from_its_second_source_is_up_to_date() {
    let fixture = CommitFixture::new();
    let archive = fixture.versioned_plugin("top-1.zip", "top", "v1");
    let first = "a".repeat(40);
    let second = "b".repeat(40);
    let mut target = manifest("top", None);
    target.version = "1.0".to_owned();
    target.sources = vec![
        PackageSource::Git {
            repository: "owner/project".to_owned(),
            commit: first,
        },
        PackageSource::Git {
            repository: "mirror/project".to_owned(),
            commit: second.clone(),
        },
    ];
    fixture.add(&target);
    let reference = CatalogReference::new("main".to_owned(), GAME.to_owned(), "top".to_owned());
    let mut store = fixture.open_store();
    store.set_history_context(
        HistoryContext::new(GAME.to_owned(), fixture.game().to_path_buf()).with_source(
            &reference,
            PackageSource::Git {
                repository: "mirror/project".to_owned(),
                commit: second.clone(),
            },
        ),
    );
    install(
        &fixture,
        &mut store,
        &archive,
        "top",
        release("top", "1.0", Vec::new()),
    );
    drop(store);
    let plan = {
        let state = fixture.readonly_store();
        plan(&fixture, &state, "top", &[])
    };

    let installed = fixture
        .readonly_store()
        .installed(fixture.game())
        .expect("state");
    assert!(matches!(
        installed[0].source,
        Some(PackageSource::Git { ref commit, .. }) if *commit == second
    ));
    assert!(update_facts(&plan).up_to_date);
    assert!(plan.pending.is_empty());
}
