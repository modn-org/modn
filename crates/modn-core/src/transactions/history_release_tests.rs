use super::catalog_tests::{reference, Fixture};
use super::history_tests::context;
use super::{CatalogConflict, CatalogDependency, CatalogRelease};

#[test]
fn release_metadata_keeps_declarations_for_a_later_uninstall() {
    let mut fixture = Fixture::new();
    let base_archive = fixture.archive("base");
    fixture
        .store
        .set_history_context(context(&fixture, "base", "base.zip"));
    let base = fixture
        .install_release(
            &base_archive,
            CatalogRelease::new(reference("base"), Vec::new()).with_version("1.0".into()),
        )
        .expect("base install");
    let archive = fixture.archive("mod");
    fixture
        .store
        .set_history_context(context(&fixture, "mod", "mod.zip"));
    let release = CatalogRelease::new(
        reference("mod"),
        vec![
            CatalogDependency::new(reference("base"), base.archive_sha256.clone())
                .with_requirement(Some(">=1.0".into())),
        ],
    )
    .with_conflicts(vec![CatalogConflict::new(
        reference("rival"),
        Some("ab".repeat(32)),
    )])
    .with_version("1.0".into());
    let package = fixture.install_release(&archive, release).expect("install");

    let operations = fixture.store.history(None).expect("history");
    let after = operations[0].packages[0]
        .release_after
        .as_ref()
        .expect("after release");
    assert_eq!(after.catalog.as_ref(), Some(&reference("mod")));
    let declarations = after.declarations.as_ref().expect("declarations");
    assert_eq!(declarations.dependencies.len(), 1);
    assert_eq!(declarations.dependencies[0].reference, reference("base"));
    assert_eq!(
        declarations.dependencies[0].requirement.as_deref(),
        Some(">=1.0")
    );
    assert_eq!(declarations.conflicts.len(), 1);
    assert_eq!(declarations.conflicts[0].reference, reference("rival"));
    assert!(!declarations.native_injector);

    fixture
        .store
        .set_history_context(context(&fixture, "mod", "mod.zip"));
    fixture.uninstall(&package.id).expect("uninstall");
    let operations = fixture.store.history(None).expect("history");
    let before = operations[0].packages[0]
        .release_before
        .as_ref()
        .expect("before release");
    assert_eq!(before.catalog.as_ref(), Some(&reference("mod")));
    assert_eq!(before.declarations, after.declarations);
}

#[test]
fn readonly_open_reads_state_without_release_declaration_columns() {
    let mut fixture = Fixture::new();
    let archive = fixture.archive("mod");
    fixture
        .store
        .set_history_context(context(&fixture, "mod", "mod.zip"));
    fixture
        .install_release(&archive, CatalogRelease::new(reference("mod"), Vec::new()))
        .expect("install");
    fixture
        .store
        .connection
        .execute_batch(
            "ALTER TABLE managed_package_releases DROP COLUMN catalog;
             ALTER TABLE managed_package_releases DROP COLUMN declarations;",
        )
        .expect("drop additive columns");
    let path = fixture.state_path();
    let before = std::fs::read(&path).expect("state bytes");
    let readonly = crate::state::Store::open_readonly(&path).expect("read only open");
    let packages = readonly.installed(&fixture.game).expect("installed");
    assert_eq!(packages.len(), 1);
    assert!(packages[0].declarations.is_none());
    assert_eq!(readonly.history(None).expect("history").len(), 1);
    drop(readonly);
    assert_eq!(std::fs::read(&path).expect("state bytes"), before);

    let store = crate::state::Store::open(&path).expect("writable open");
    let columns: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('managed_package_releases')
             WHERE name IN ('catalog', 'declarations')",
            [],
            |row| row.get(0),
        )
        .expect("column count");
    assert_eq!(columns, 2);
}
