use crate::acquisition::ReleaseDigest;
use crate::transactions::catalog_tests::{reference, Fixture};
use crate::transactions::{CatalogConflict, CatalogRelease};

#[test]
fn an_unknown_release_digest_matches_no_recorded_empty_digest() {
    let mut fixture = Fixture::new();
    let installed_archive = fixture.archive("installed");
    let installed_release =
        CatalogRelease::new(reference("installed"), Vec::new()).with_conflicts(vec![
            CatalogConflict::new(reference("other"), Some(String::new())),
        ]);
    let installed = fixture
        .install_release(&installed_archive, installed_release)
        .expect("install");

    let stored: String = fixture
        .store
        .connection
        .query_row("SELECT archive_blake3 FROM package_conflicts", [], |row| {
            row.get(0)
        })
        .expect("recorded conflict digest");
    assert_eq!(stored, "", "the fixture records an empty conflict digest");

    let candidate = CatalogRelease::new(reference("git-release"), Vec::new());
    let found = fixture
        .store
        .conflicting_packages_for_digest(&fixture.game, Some(&candidate), &ReleaseDigest::Unknown)
        .expect("conflict check");
    assert!(
        found.is_empty(),
        "an unknown digest must not match an empty digest: {found:?}"
    );

    let error = fixture
        .store
        .conflicting_packages(&fixture.game, Some(&candidate), "")
        .expect_err("an empty digest is not a digest");
    assert!(error.to_string().contains("BLAKE3"), "{error}");

    let by_reference = CatalogRelease::new(reference("git-release"), Vec::new())
        .with_conflicts(vec![CatalogConflict::new(reference("installed"), None)]);
    let found = fixture
        .store
        .conflicting_packages_for_digest(
            &fixture.game,
            Some(&by_reference),
            &ReleaseDigest::Unknown,
        )
        .expect("conflict check");
    assert_eq!(
        found,
        [format!("{} (deps/test-game/installed)", installed.id)]
    );
}
