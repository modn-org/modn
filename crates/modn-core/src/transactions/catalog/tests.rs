use super::{CatalogDependency, CatalogReference, CatalogRelease, DependencyState};
use crate::transactions::catalog_tests::{reference, Fixture};

#[test]
#[should_panic(expected = "a catalog archive digest is 64 lowercase hex characters")]
fn a_catalog_dependency_refuses_an_empty_digest() {
    let reference =
        CatalogReference::new("deps".to_owned(), "test-game".to_owned(), "base".to_owned());
    let _ = CatalogDependency::new(reference, String::new());
}

#[test]
fn an_unknown_dependency_digest_matches_by_recorded_provenance() {
    let mut fixture = Fixture::new();
    let archive = fixture.archive("base");
    let installed = fixture
        .install_release(&archive, CatalogRelease::new(reference("base"), Vec::new()))
        .expect("install");
    let dependency = CatalogDependency::unknown(reference("base"));

    let states = fixture
        .store
        .dependency_states(&fixture.game, &[dependency])
        .expect("dependency states");

    match states.as_slice() {
        [DependencyState::Ready { package_ids }] => {
            assert_eq!(package_ids.len(), 1);
            assert_eq!(package_ids[0], installed.id);
        }
        other => panic!("expected the recorded provenance to be ready: {other:?}"),
    }
}
