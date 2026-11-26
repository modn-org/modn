use super::super::catalog::CatalogRelease;
use super::super::catalog_tests::{plan, plan_into, reference, Fixture};
use super::super::{NativeInjectorNotAllowed, NativeInjectorPolicy};
use fs2::FileExt;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

fn table_counts(fixture: &Fixture) -> Vec<(String, i64)> {
    [
        "managed_packages",
        "managed_files",
        "managed_directories",
        "installation_roots",
        "catalog_provenance",
        "package_dependencies",
        "package_conflicts",
        "package_requirements",
        "pending_transactions",
        "pending_files",
        "pending_directories",
        "pending_links",
        "pending_stacks",
        "pending_stack_packages",
    ]
    .into_iter()
    .map(|table| (table.to_owned(), fixture.count(table)))
    .collect()
}

fn tree(root: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
    let mut entries = Vec::new();
    collect_tree(root, root, &mut entries);
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}

fn collect_tree(root: &Path, current: &Path, entries: &mut Vec<(PathBuf, Option<Vec<u8>>)>) {
    for entry in fs::read_dir(current).unwrap() {
        let path = entry.unwrap().path();
        let relative = path.strip_prefix(root).unwrap().to_path_buf();
        if path.is_dir() {
            entries.push((relative, None));
            collect_tree(root, &path, entries);
        } else {
            entries.push((relative, Some(fs::read(&path).unwrap())));
        }
    }
}

fn database(fixture: &Fixture) -> Vec<u8> {
    fs::read(fixture.state_path()).unwrap()
}

fn contender(lock_path: &Path) -> fs::File {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(lock_path)
        .unwrap()
}

#[test]
fn planned_entry_point_installs_the_plan_the_closure_returns() {
    let mut fixture = Fixture::new();
    let archive = fixture.archive("planned");
    let game = fixture.game.clone();
    let lock_path = fixture.state_path().with_file_name(".transactions.lock");
    let pre_lock = plan(&game, &archive);
    assert_eq!(pre_lock.operations[0].destination, "addons/planned.plugin");
    let release = CatalogRelease::new(reference("planned"), Vec::new());
    let mut calls = 0;
    let package = fixture
        .store
        .install_catalog_release_planned(
            &game,
            &archive,
            &release,
            NativeInjectorPolicy::Refuse,
            &[],
            |planned_archive| {
                calls += 1;
                assert_eq!(planned_archive, archive.as_path());
                assert!(
                    contender(&lock_path).try_lock_exclusive().is_err(),
                    "the transaction lock must be held while the closure runs"
                );
                Ok(plan_into(&game, planned_archive, "replanned"))
            },
        )
        .unwrap();

    assert_eq!(calls, 1);
    assert_eq!(package.files.len(), 1);
    assert_eq!(package.files[0].destination, "replanned/planned.plugin");
    assert!(game.join("replanned/planned.plugin").is_file());
    assert!(!game.join("addons/planned.plugin").exists());
    assert_eq!(fixture.store.installed(&game).unwrap().len(), 1);
    assert_eq!(fixture.count("managed_packages"), 1);
    assert_eq!(fixture.count("managed_files"), 1);
    assert_eq!(fixture.count("catalog_provenance"), 1);
    assert!(fixture.store.pending().unwrap().is_empty());
}

#[test]
fn closure_error_writes_nothing_and_propagates_unchanged() {
    let mut fixture = Fixture::new();
    let archive = fixture.archive("aborted");
    let game = fixture.game.clone();
    let release = CatalogRelease::new(reference("aborted"), Vec::new());
    let tree_before = tree(&game);
    let database_before = database(&fixture);
    let tables_before = table_counts(&fixture);

    let error = fixture
        .store
        .install_catalog_release_planned(
            &game,
            &archive,
            &release,
            NativeInjectorPolicy::Refuse,
            &[],
            |_| anyhow::bail!("injected re-plan failure"),
        )
        .unwrap_err();

    assert_eq!(error.to_string(), "injected re-plan failure");
    assert_eq!(tree(&game), tree_before);
    assert_eq!(database(&fixture), database_before);
    assert_eq!(table_counts(&fixture), tables_before);
    assert!(fixture.store.installed(&game).unwrap().is_empty());
    assert!(fixture.store.pending().unwrap().is_empty());
    assert_eq!(fixture.store.recover().unwrap(), 0);
}

#[test]
fn pending_journal_refuses_before_the_closure_runs() {
    let mut fixture = Fixture::new();
    let archive = fixture.archive("blocked");
    let game = fixture.game.clone();
    let release = CatalogRelease::new(reference("blocked"), Vec::new());
    fixture
        .store
        .insert_journal(
            "pending-planned",
            "install",
            &game,
            "pending-planned",
            None,
            &[],
            &[],
        )
        .unwrap();
    let mut ran = false;

    let error = fixture
        .store
        .install_catalog_release_planned(
            &game,
            &archive,
            &release,
            NativeInjectorPolicy::Refuse,
            &[],
            |planned_archive| {
                ran = true;
                Ok(plan(&game, planned_archive))
            },
        )
        .unwrap_err();

    assert!(!ran, "the idle check must refuse before the closure runs");
    assert!(
        error
            .to_string()
            .contains("unfinished transaction detected"),
        "{error:#}"
    );
    assert_eq!(fixture.count("pending_transactions"), 1);
    assert!(fixture.store.installed(&game).unwrap().is_empty());
    assert_eq!(fixture.store.recover().unwrap(), 1);
    assert_eq!(fixture.count("pending_transactions"), 0);
}

#[test]
fn missing_archive_refuses_before_the_closure_runs() {
    let mut fixture = Fixture::new();
    let archive = fixture.game.join("missing.zip");
    let game = fixture.game.clone();
    let release = CatalogRelease::new(reference("missing"), Vec::new());
    let mut ran = false;

    let error = fixture
        .store
        .install_catalog_release_planned(
            &game,
            &archive,
            &release,
            NativeInjectorPolicy::Refuse,
            &[],
            |planned_archive| {
                ran = true;
                Ok(plan(&game, planned_archive))
            },
        )
        .unwrap_err();

    assert!(
        !ran,
        "the archive inspection must refuse before the closure runs"
    );
    assert!(error.to_string().contains("missing.zip"), "{error:#}");
    assert!(fixture.store.pending().unwrap().is_empty());
}

#[test]
fn native_injector_without_policy_refuses_inside_the_entry_point() {
    let mut fixture = Fixture::new();
    let archive = fixture.archive("injector");
    let game = fixture.game.clone();
    let release = CatalogRelease::new(reference("injector"), Vec::new()).with_native_injector(true);
    let tree_before = tree(&game);
    let mut ran = false;

    let error = fixture
        .store
        .install_catalog_release_planned(
            &game,
            &archive,
            &release,
            NativeInjectorPolicy::Refuse,
            &[],
            |planned_archive| {
                ran = true;
                Ok(plan(&game, planned_archive))
            },
        )
        .unwrap_err();

    assert!(
        !ran,
        "the native-injector policy refuses before re-planning"
    );
    let refusal = error
        .downcast_ref::<NativeInjectorNotAllowed>()
        .expect("typed native-injector refusal");
    assert_eq!(refusal.code(), "NATIVE_INJECTOR_NOT_ALLOWED");
    assert_eq!(tree(&game), tree_before);
    assert!(fixture.store.pending().unwrap().is_empty());
}
