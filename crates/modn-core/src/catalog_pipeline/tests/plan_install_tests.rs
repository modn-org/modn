//! Planning a whole catalog install from supplied data.

use super::fixtures::{self, plan_for, readonly_store, reference_of, release, tree, Fixture, GAME};
use crate::approval::Capabilities;
use crate::catalog_pipeline::{preview_catalog_install, PlanKind, PlanRefusal};
use crate::state::Store;
use crate::transactions::{DependencyState, NativeInjectorPolicy};
use std::fs;

#[test]
fn a_plain_install_plans_one_release() {
    let fixture = Fixture::new();
    let game = fixture.game();
    let archive = fixture.zip("top");
    let pin = fixtures::digest(&archive);
    fixture.add(&fixtures::manifest("top", Some(&pin)));
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

    assert_eq!(plan.kind, PlanKind::Install);
    assert_eq!(plan.selected.to_string(), reference_of("top"));
    assert_eq!(plan.game_id, GAME);
    assert_eq!(plan.installation, game);
    assert_eq!(plan.order.len(), 1);
    assert_eq!(plan.order[0].package.package.id, "top");
    assert_eq!(plan.order[0].release.version.as_deref(), Some("1.0"));
    assert_eq!(plan.order[0].digest.as_str(), Some(pin.as_str()));
    assert!(plan.order[0].state.is_none());
    assert!(plan.order[0].served.is_some());
    assert!(plan.pending.is_empty());
    assert!(plan.conflicts.is_empty());
    assert_eq!(plan.native_injector, NativeInjectorPolicy::Refuse);
    assert!(plan.refusals.is_empty());
}

#[test]
fn a_dependency_chain_reports_ready_and_missing_dependencies() {
    let fixture = Fixture::new();
    let game = fixture.game();
    let base_archive = fixture.zip("base");
    let mid_archive = fixture.zip("mid");
    let top_archive = fixture.zip("top");
    let mut mid = fixtures::manifest("mid", Some(&fixtures::digest(&mid_archive)));
    mid.dependencies = vec!["base".to_owned()];
    let mut top = fixtures::manifest("top", Some(&fixtures::digest(&top_archive)));
    top.dependencies = vec!["mid".to_owned()];
    fixture.add(&fixtures::manifest(
        "base",
        Some(&fixtures::digest(&base_archive)),
    ));
    fixture.add(&mid);
    fixture.add(&top);
    let snapshot = fixture.snapshot();
    let mut store = Store::open(&fixture.state_path()).expect("state");
    fixture.install(&mut store, &game, &base_archive, &release("base"));
    drop(store);

    let plan = plan_for(
        &fixture,
        &snapshot,
        &game,
        "top",
        Some(&readonly_store(&fixture)),
        &[],
        Capabilities::default(),
    );

    let ids: Vec<&str> = plan
        .order
        .iter()
        .map(|planned| planned.package.package.id.as_str())
        .collect();
    assert_eq!(ids, ["base", "mid", "top"]);
    assert!(matches!(
        plan.order[0].state,
        Some(DependencyState::Ready { .. })
    ));
    assert_eq!(plan.order[1].state, Some(DependencyState::Missing));
    assert!(plan.order[2].state.is_none());
    let pending: Vec<String> = plan
        .pending
        .iter()
        .map(|pending| pending.reference.to_string())
        .collect();
    assert_eq!(pending, [reference_of("mid"), reference_of("top")]);
    assert!(plan.refusals.is_empty());
}

#[test]
fn a_damaged_dependency_is_not_ready() {
    let fixture = Fixture::new();
    let game = fixture.game();
    let base_archive = fixture.zip("base");
    let mut top = fixtures::manifest("top", Some(&"a".repeat(64)));
    top.dependencies = vec!["base".to_owned()];
    fixture.add(&fixtures::manifest(
        "base",
        Some(&fixtures::digest(&base_archive)),
    ));
    fixture.add(&top);
    let snapshot = fixture.snapshot();
    let mut store = Store::open(&fixture.state_path()).expect("state");
    fixture.install(&mut store, &game, &base_archive, &release("base"));
    drop(store);
    fs::write(game.join("addons/base.txt"), b"modified").expect("drift");

    let plan = plan_for(
        &fixture,
        &snapshot,
        &game,
        "top",
        Some(&readonly_store(&fixture)),
        &[],
        Capabilities::default(),
    );

    assert!(matches!(
        plan.order[0].state,
        Some(DependencyState::Damaged { .. })
    ));
    assert!(plan.refusals.iter().any(|refusal| matches!(
        refusal,
        PlanRefusal::DependencyNotReady { reference, state: DependencyState::Damaged { .. } }
            if *reference == fixtures::package(&snapshot, "base").reference()
    )));
    assert!(!plan
        .pending
        .iter()
        .any(|pending| pending.reference.to_string() == reference_of("base")));
}

#[test]
fn a_dependency_installed_from_another_release_is_not_ready() {
    let fixture = Fixture::new();
    let game = fixture.game();
    let base_archive = fixture.zip("base");
    let other_archive = fixture.zip("other");
    let mut top = fixtures::manifest("top", Some(&"a".repeat(64)));
    top.dependencies = vec!["base".to_owned()];
    fixture.add(&fixtures::manifest(
        "base",
        Some(&fixtures::digest(&base_archive)),
    ));
    fixture.add(&top);
    let snapshot = fixture.snapshot();
    let mut store = Store::open(&fixture.state_path()).expect("state");
    fixture.install(&mut store, &game, &other_archive, &release("base"));
    drop(store);

    let plan = plan_for(
        &fixture,
        &snapshot,
        &game,
        "top",
        Some(&readonly_store(&fixture)),
        &[],
        Capabilities::default(),
    );

    assert!(matches!(
        plan.order[0].state,
        Some(DependencyState::OtherRelease { .. })
    ));
    assert!(plan.refusals.iter().any(|refusal| matches!(
        refusal,
        PlanRefusal::DependencyNotReady {
            state: DependencyState::OtherRelease { .. },
            ..
        }
    )));
}

#[test]
fn a_declared_conflict_with_an_installed_package_is_reported() {
    let fixture = Fixture::new();
    let game = fixture.game();
    let enemy_archive = fixture.zip("enemy");
    let mut top = fixtures::manifest("top", Some(&"a".repeat(64)));
    top.conflicts = vec!["enemy".to_owned()];
    fixture.add(&fixtures::manifest("enemy", Some(&"b".repeat(64))));
    fixture.add(&top);
    let snapshot = fixture.snapshot();
    let mut store = Store::open(&fixture.state_path()).expect("state");
    fixture.install(&mut store, &game, &enemy_archive, &release("enemy"));
    drop(store);

    let plan = plan_for(
        &fixture,
        &snapshot,
        &game,
        "top",
        Some(&readonly_store(&fixture)),
        &[],
        Capabilities::default(),
    );

    assert_eq!(plan.conflicts.len(), 1);
    assert_eq!(plan.conflicts[0].reference.to_string(), reference_of("top"));
    assert!(plan.conflicts[0].installed[0].ends_with("(main/test-game/enemy)"));
    assert!(plan.refusals.iter().any(|refusal| matches!(
        refusal,
        PlanRefusal::Conflict { conflict } if conflict.reference.to_string() == reference_of("top")
    )));
}

#[test]
fn planning_writes_no_manager_state() {
    let fixture = Fixture::new();
    let game = fixture.game();
    let base_archive = fixture.zip("base");
    let mut top = fixtures::manifest("top", Some(&"a".repeat(64)));
    top.dependencies = vec!["base".to_owned()];
    top.native_injector = true;
    fixture.add(&fixtures::manifest(
        "base",
        Some(&fixtures::digest(&base_archive)),
    ));
    fixture.add(&top);
    let snapshot = fixture.snapshot();
    let mut store = Store::open(&fixture.state_path()).expect("state");
    fixture.install(&mut store, &game, &base_archive, &release("base"));
    store
        .record_approval(
            &game,
            "main",
            "top",
            &Capabilities::new(true, Vec::<String>::new()),
        )
        .expect("approval");
    drop(store);
    let store = readonly_store(&fixture);

    let data_before = tree(fixture.data());
    let profile_before = tree(fixture.roots().profile());
    let plan = plan_for(
        &fixture,
        &snapshot,
        &game,
        "top",
        Some(&store),
        &[],
        Capabilities::default(),
    );

    assert!(plan.refusals.is_empty());
    assert_eq!(tree(fixture.data()), data_before);
    assert_eq!(tree(fixture.roots().profile()), profile_before);
}

#[test]
fn releases_that_share_one_archive_are_members_once() {
    let fixture = Fixture::new();
    let game = fixture.game();
    let shared = fixture.zip("shared");
    let pin = fixtures::digest(&shared);
    let mut top = fixtures::manifest("top", Some(&fixtures::digest(&fixture.zip("top"))));
    top.dependencies = vec!["a".to_owned(), "b".to_owned()];
    fixture.add(&fixtures::manifest("a", Some(&pin)));
    fixture.add(&fixtures::manifest("b", Some(&pin)));
    fixture.add(&top);
    let snapshot = fixture.snapshot();
    let plan = plan_for(
        &fixture,
        &snapshot,
        &game,
        "top",
        None,
        std::slice::from_ref(&shared),
        Capabilities::default(),
    );

    let members = plan.members().expect("members");

    assert_eq!(
        members
            .iter()
            .map(|planned| planned.package.package.id.as_str())
            .collect::<Vec<_>>(),
        ["a", "top"]
    );
}

#[test]
fn releases_that_share_one_archive_must_declare_the_same_requirements() {
    let fixture = Fixture::new();
    let game = fixture.game();
    let shared = fixture.zip("shared");
    let pin = fixtures::digest(&shared);
    let mut a = fixtures::manifest("a", Some(&pin));
    a.conflicts = vec!["rival".to_owned()];
    let mut top = fixtures::manifest("top", Some(&fixtures::digest(&fixture.zip("top"))));
    top.dependencies = vec!["a".to_owned(), "b".to_owned()];
    fixture.add(&a);
    fixture.add(&fixtures::manifest("b", Some(&pin)));
    fixture.add(&top);
    let snapshot = fixture.snapshot();
    let plan = plan_for(
        &fixture,
        &snapshot,
        &game,
        "top",
        None,
        std::slice::from_ref(&shared),
        Capabilities::default(),
    );

    let error = plan.members().expect_err("different declarations");

    assert_eq!(
        error.to_string(),
        format!(
            "main/{GAME}/a and main/{GAME}/b share one archive but declare different dependencies or conflicts; install them separately"
        )
    );
}

#[test]
fn the_selected_release_cannot_share_an_archive_with_a_dependency() {
    let fixture = Fixture::new();
    let game = fixture.game();
    let shared = fixture.zip("shared");
    let pin = fixtures::digest(&shared);
    let mut top = fixtures::manifest("top", Some(&pin));
    top.dependencies = vec!["a".to_owned()];
    fixture.add(&fixtures::manifest("a", Some(&pin)));
    fixture.add(&top);
    let snapshot = fixture.snapshot();
    let plan = plan_for(
        &fixture,
        &snapshot,
        &game,
        "top",
        None,
        std::slice::from_ref(&shared),
        Capabilities::default(),
    );

    let error = plan.members().expect_err("shared selected archive");

    assert_eq!(
        error.to_string(),
        format!(
            "main/{GAME}/top: its archive is also the archive of a missing dependency; install that dependency first"
        )
    );
}

#[test]
fn a_preview_skips_members_without_an_archive() {
    let fixture = Fixture::new();
    let game = fixture.game();
    let archive = fixture.zip("top");
    fixture.add(&fixtures::manifest(
        "top",
        Some(&fixtures::digest(&archive)),
    ));
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

    let previews = preview_catalog_install(
        &fixture.roots(),
        &plan,
        &std::collections::BTreeMap::new(),
        &[],
    )
    .expect("previews");

    assert!(previews.is_empty());
}
