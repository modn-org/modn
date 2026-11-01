//! Declared conflicts recorded with ownership and checked under the lock.
use super::catalog_tests::{reference, Fixture};
use super::{CatalogConflict, CatalogRelease};

#[test]
fn declared_conflicts_block_installation_in_either_order() {
    let mut fixture = Fixture::new();
    let a_archive = fixture.archive("a");
    let b_archive = fixture.archive("b");
    let a = || {
        CatalogRelease::new(reference("a"), Vec::new())
            .with_conflicts(vec![CatalogConflict::new(reference("b"), None)])
    };
    let b = || CatalogRelease::new(reference("b"), Vec::new());

    // `a` declares the conflict and is installed first: `b` is refused.
    let installed = fixture.install_release(&a_archive, a()).unwrap();
    assert_eq!(fixture.count("package_conflicts"), 1);
    let error = fixture
        .install_release(&b_archive, b())
        .unwrap_err()
        .to_string();
    assert!(
        error.contains(&format!(
            "deps/test-game/b conflicts with installed {} (deps/test-game/a)",
            installed.id
        )),
        "{error}"
    );
    assert!(!fixture.game.join("addons/b.plugin").exists());
    fixture.uninstall(&installed.id).unwrap();
    assert_eq!(fixture.count("package_conflicts"), 0);

    // `b` is installed first: `a`, which declares the conflict, is refused.
    let installed = fixture.install_release(&b_archive, b()).unwrap();
    let error = fixture
        .install_release(&a_archive, a())
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("deps/test-game/a conflicts with installed"),
        "{error}"
    );
    assert!(error.contains(&installed.id), "{error}");
    assert!(!fixture.game.join("addons/a.plugin").exists());
    assert_eq!(fixture.count("pending_transactions"), 0);
}

#[test]
fn a_conflict_also_matches_the_conflicting_release_by_digest() {
    let mut fixture = Fixture::new();
    let shared = fixture.archive("shared");
    let digest = crate::archive::inspect(&shared).unwrap().sha256;
    let rival = fixture.archive("rival");
    // `b` is published with the same archive as `a`, which is installed.
    let b = || CatalogConflict::new(reference("b"), Some(digest.clone()));
    let installed = fixture
        .install_release(&shared, CatalogRelease::new(reference("a"), Vec::new()))
        .unwrap();
    let error = fixture
        .install_release(
            &rival,
            CatalogRelease::new(reference("rival"), Vec::new()).with_conflicts(vec![b()]),
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains(&installed.id), "{error}");
    fixture.uninstall(&installed.id).unwrap();

    // Reverse: the installed `rival` declares `b`; `a` has `b`'s archive.
    let installed = fixture
        .install_release(
            &rival,
            CatalogRelease::new(reference("rival"), Vec::new()).with_conflicts(vec![b()]),
        )
        .unwrap();
    let error = fixture
        .install_release(&shared, CatalogRelease::new(reference("a"), Vec::new()))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("deps/test-game/a conflicts with installed"),
        "{error}"
    );
    assert!(error.contains(&installed.id), "{error}");
    assert!(!fixture.game.join("addons/shared.plugin").exists());
}

#[test]
fn legacy_read_only_state_still_matches_conflicts_by_digest() {
    let mut fixture = Fixture::new();
    let shared = fixture.archive("shared");
    let digest = crate::archive::inspect(&shared).unwrap().sha256;
    let installed = fixture
        .install_release(&shared, CatalogRelease::new(reference("a"), Vec::new()))
        .unwrap();
    // Simulate state written before catalog provenance and conflicts existed.
    fixture
        .store
        .connection
        .execute_batch(
            "DROP TABLE package_dependencies; DROP TABLE package_conflicts;
             DROP TABLE pending_stack_packages; DROP TABLE catalog_provenance;",
        )
        .unwrap();
    let store = crate::state::Store::open_readonly(&fixture.state_path()).unwrap();
    let rival = CatalogRelease::new(reference("rival"), Vec::new())
        .with_conflicts(vec![CatalogConflict::new(reference("b"), Some(digest))]);
    let found = store
        .conflicting_packages(&fixture.game, Some(&rival), &"0".repeat(64))
        .unwrap();
    assert_eq!(
        found,
        [format!(
            "{} (archive {})",
            installed.id, installed.archive_sha256
        )]
    );
}
