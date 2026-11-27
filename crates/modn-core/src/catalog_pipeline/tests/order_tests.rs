//! Install order resolution.

use super::fixtures::{self, Fixture};
use crate::catalog_pipeline::order;
#[test]
fn a_chain_is_resolved_dependencies_first_with_the_selected_last() {
    let fixture = Fixture::new();
    let pin = "a".repeat(64);
    let mut top = fixtures::manifest("top", Some(&pin));
    top.dependencies = vec!["mid".to_owned()];
    let mut mid = fixtures::manifest("mid", Some(&pin));
    mid.dependencies = vec!["base".to_owned()];
    fixture.add(&fixtures::manifest("base", Some(&pin)));
    fixture.add(&mid);
    fixture.add(&top);
    let snapshot = fixture.snapshot();
    let selected = fixtures::package(&snapshot, "top");

    let order = order::resolve(&snapshot, &selected).expect("order");

    let ids: Vec<&str> = order
        .iter()
        .map(|ordered| ordered.package.package.id.as_str())
        .collect();
    assert_eq!(ids, ["base", "mid", "top"]);
    assert_eq!(order.last().expect("selected").package.package.id, "top");
    assert_eq!(order[2].release.reference.mod_id, "top");
    assert_eq!(order[2].release.version.as_deref(), Some("1.0"));
    assert_eq!(order[2].release.dependencies.len(), 1);
    assert_eq!(order[2].release.dependencies[0].reference.mod_id, "mid");
}

#[test]
fn a_diamond_places_the_shared_dependency_once_before_every_dependent() {
    let fixture = Fixture::new();
    let pin = "a".repeat(64);
    let mut top = fixtures::manifest("top", Some(&pin));
    top.dependencies = vec!["left".to_owned(), "right".to_owned()];
    let mut left = fixtures::manifest("left", Some(&pin));
    left.dependencies = vec!["base".to_owned()];
    let mut right = fixtures::manifest("right", Some(&pin));
    right.dependencies = vec!["base".to_owned()];
    fixture.add(&fixtures::manifest("base", Some(&pin)));
    fixture.add(&left);
    fixture.add(&right);
    fixture.add(&top);
    let snapshot = fixture.snapshot();
    let selected = fixtures::package(&snapshot, "top");

    let order = order::resolve(&snapshot, &selected).expect("order");

    let ids: Vec<&str> = order
        .iter()
        .map(|ordered| ordered.package.package.id.as_str())
        .collect();
    assert_eq!(ids, ["base", "left", "right", "top"]);
    assert_eq!(order.last().expect("selected").package.package.id, "top");
}

#[test]
fn a_missing_dependency_fails_with_the_install_order_error() {
    let fixture = Fixture::new();
    let pin = "a".repeat(64);
    let mut top = fixtures::manifest("top", Some(&pin));
    top.dependencies = vec!["base".to_owned()];
    fixture.add(&top);
    let snapshot = fixture.snapshot();
    let selected = fixtures::package(&snapshot, "top");

    let error = order::resolve(&snapshot, &selected).expect_err("missing dependency");

    assert_eq!(
        error.to_string(),
        "dependency unavailable in bucket: main/test-game/base (required by top)"
    );
}
