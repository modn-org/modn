//! Refusals of one catalog install plan, as data in today's check order.

use super::fixtures::{
    self, plan_for, readonly_store, reference_of, refusal_label, release, Fixture,
};
use crate::approval::Capabilities;
use crate::buckets::PackageSource;
use crate::catalog_pipeline::PlanRefusal;
use crate::error::ErrorCode;
use crate::state::Store;
use crate::transactions::NativeInjectorPolicy;
use std::fs;

#[test]
fn a_native_injector_is_refused_without_a_flag_or_a_stored_approval() {
    let fixture = Fixture::new();
    let game = fixture.game();
    let mut manifest = fixtures::manifest("injector", Some(&"a".repeat(64)));
    manifest.native_injector = true;
    fixture.add(&manifest);
    let snapshot = fixture.snapshot();

    let plan = plan_for(
        &fixture,
        &snapshot,
        &game,
        "injector",
        None,
        &[],
        Capabilities::default(),
    );

    assert_eq!(plan.native_injector, NativeInjectorPolicy::Refuse);
    assert!(plan.refusals.iter().any(|refusal| matches!(
        refusal,
        PlanRefusal::NativeInjector { reference }
            if reference.to_string() == reference_of("injector")
    )));
}

#[test]
fn a_native_injector_is_accepted_with_the_flag() {
    let fixture = Fixture::new();
    let game = fixture.game();
    let mut manifest = fixtures::manifest("injector", Some(&"a".repeat(64)));
    manifest.native_injector = true;
    fixture.add(&manifest);
    let snapshot = fixture.snapshot();

    let plan = plan_for(
        &fixture,
        &snapshot,
        &game,
        "injector",
        None,
        &[],
        Capabilities::new(true, Vec::<String>::new()),
    );

    assert_eq!(plan.native_injector, NativeInjectorPolicy::Allow);
    assert!(plan.refusals.is_empty());
}

#[test]
fn a_stored_approval_accepts_a_native_injector() {
    let fixture = Fixture::new();
    let game = fixture.game();
    let mut manifest = fixtures::manifest("injector", Some(&"a".repeat(64)));
    manifest.native_injector = true;
    fixture.add(&manifest);
    let snapshot = fixture.snapshot();
    let mut store = Store::open(&fixture.state_path()).expect("state");
    store
        .record_approval(
            &game,
            "main",
            "injector",
            &Capabilities::new(true, Vec::<String>::new()),
        )
        .expect("approval");
    drop(store);

    let plan = plan_for(
        &fixture,
        &snapshot,
        &game,
        "injector",
        Some(&readonly_store(&fixture)),
        &[],
        Capabilities::default(),
    );

    assert_eq!(plan.native_injector, NativeInjectorPolicy::Allow);
    assert!(plan.order[0]
        .stored
        .as_ref()
        .is_some_and(|stored| stored.native_injector));
    assert!(plan.refusals.is_empty());
}

#[test]
fn an_uncovered_executable_needs_approval() {
    let fixture = Fixture::new();
    let game = fixture.game();
    let mut manifest = fixtures::manifest("tool", Some(&"a".repeat(64)));
    manifest.executables = vec!["engine/tools/scc.exe".to_owned()];
    fixture.add(&manifest);
    let snapshot = fixture.snapshot();

    let plan = plan_for(
        &fixture,
        &snapshot,
        &game,
        "tool",
        None,
        &[],
        Capabilities::default(),
    );

    let refusal = plan
        .refusals
        .iter()
        .find_map(|refusal| match refusal {
            PlanRefusal::ApprovalRequired { refusal } => Some(refusal),
            _ => None,
        })
        .expect("approval refusal");
    assert_eq!(refusal.code(), ErrorCode::ApprovalRequired);
    assert!(refusal.to_string().contains("engine/tools/scc.exe"));

    let allowed = plan_for(
        &fixture,
        &snapshot,
        &game,
        "tool",
        None,
        &[],
        Capabilities::new(false, ["engine/tools/scc.exe"]),
    );
    assert!(allowed.refusals.is_empty());
}

#[test]
fn a_supplied_archive_that_matches_nothing_is_reported() {
    let fixture = Fixture::new();
    let game = fixture.game();
    let archive = fixture.zip("other");
    fixture.add(&fixtures::manifest("top", Some(&"a".repeat(64))));
    let snapshot = fixture.snapshot();

    let plan = plan_for(
        &fixture,
        &snapshot,
        &game,
        "top",
        None,
        std::slice::from_ref(&archive),
        Capabilities::default(),
    );

    assert!(plan.order[0].served.is_none());
    assert_eq!(plan.pending.len(), 1);
    assert!(plan.refusals.iter().any(|refusal| matches!(
        refusal,
        PlanRefusal::ArchiveUnmatched { path } if *path == archive
    )));
}

#[test]
fn a_missing_archive_without_a_downloadable_source_is_refused() {
    let fixture = Fixture::new();
    let game = fixture.game();
    let mut manifest = fixtures::manifest("manual", Some(&"a".repeat(64)));
    manifest.sources = vec![PackageSource::Manual {
        filename: "manual.zip".to_owned(),
    }];
    fixture.add(&manifest);
    let snapshot = fixture.snapshot();

    let plan = plan_for(
        &fixture,
        &snapshot,
        &game,
        "manual",
        None,
        &[],
        Capabilities::default(),
    );

    assert_eq!(plan.pending.len(), 1);
    assert!(!plan.pending[0].downloadable);
    assert!(matches!(
        &plan.refusals[0],
        PlanRefusal::ArchiveRequired { reference, source }
            if reference.to_string() == reference_of("manual")
                && matches!(source, PackageSource::Manual { .. })
    ));
}

#[test]
fn a_release_without_an_archive_still_needs_a_download() {
    let fixture = Fixture::new();
    let game = fixture.game();
    fixture.add(&fixtures::manifest("top", Some(&"a".repeat(64))));
    let snapshot = fixture.snapshot();

    let plan = plan_for(
        &fixture,
        &snapshot,
        &game,
        "top",
        None,
        &[],
        Capabilities::default(),
    );

    assert_eq!(plan.pending.len(), 1);
    assert!(plan.pending[0].downloadable);
    assert!(!plan
        .refusals
        .iter()
        .any(|refusal| matches!(refusal, PlanRefusal::ArchiveRequired { .. })));
}

#[test]
fn refusals_follow_the_order_the_install_command_checks_them() {
    let fixture = Fixture::new();
    let game = fixture.game();
    let broken_archive = fixture.zip("broken");
    let enemy_archive = fixture.zip("enemy");
    let mut dep = fixtures::manifest("dep", Some(&"b".repeat(64)));
    dep.conflicts = vec!["enemy".to_owned()];
    dep.sources = vec![PackageSource::Manual {
        filename: "dep.zip".to_owned(),
    }];
    let mut top = fixtures::manifest("top", Some(&"c".repeat(64)));
    top.dependencies = vec!["broken".to_owned(), "dep".to_owned()];
    top.executables = vec!["engine/tools/scc.exe".to_owned()];
    top.native_injector = true;
    fixture.add(&fixtures::manifest(
        "broken",
        Some(&fixtures::digest(&broken_archive)),
    ));
    fixture.add(&fixtures::manifest("enemy", Some(&"d".repeat(64))));
    fixture.add(&dep);
    fixture.add(&top);
    let snapshot = fixture.snapshot();
    let mut store = Store::open(&fixture.state_path()).expect("state");
    fixture.install(&mut store, &game, &broken_archive, &release("broken"));
    fixture.install(&mut store, &game, &enemy_archive, &release("enemy"));
    drop(store);
    fs::write(game.join("addons/broken.txt"), b"modified").expect("drift");
    let unmatched = fixture.zip("other");

    let plan = plan_for(
        &fixture,
        &snapshot,
        &game,
        "top",
        Some(&readonly_store(&fixture)),
        std::slice::from_ref(&unmatched),
        Capabilities::default(),
    );

    let labels: Vec<String> = plan.refusals.iter().map(refusal_label).collect();
    assert_eq!(
        labels,
        [
            format!("native {}", reference_of("top")),
            format!("unmatched {}", unmatched.display()),
            format!("conflict {}", reference_of("dep")),
            format!("dependency {}", reference_of("broken")),
            format!("archive {}", reference_of("dep")),
            format!("approval {}", reference_of("top")),
        ]
    );
}
