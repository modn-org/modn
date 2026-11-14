use super::catalog_tests::{module_source, plan, reference, Fixture};
use super::install::Fault;
use super::{CatalogRelease, NativeInjectorPolicy, StackMember};
use crate::archive::inspect;
use crate::buckets::PackageSource;
use crate::game_modules::with_module_source;
use crate::history::{HistoryContext, HistoryKind};
use crate::progress::{CancellationToken, OperationControl};
use crate::state::Store;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;

fn source(filename: &str) -> PackageSource {
    PackageSource::Manual {
        filename: filename.to_owned(),
    }
}

pub(super) fn context(fixture: &Fixture, mod_id: &str, filename: &str) -> HistoryContext {
    HistoryContext::new("test-game", fixture.game.clone())
        .with_source(&reference(mod_id), source(filename))
}

fn archive_with(directory: &Path, name: &str, contents: &[u8]) -> PathBuf {
    let path = directory.join(format!("{name}.zip"));
    let mut zip = zip::ZipWriter::new(File::create(&path).expect("archive"));
    zip.start_file("plugin/base.plugin", SimpleFileOptions::default())
        .expect("entry");
    zip.write_all(contents).expect("payload");
    zip.finish().expect("finish");
    path
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[test]
fn install_and_uninstall_record_releases_sources_and_digests() {
    let mut fixture = Fixture::new();
    let archive = fixture.archive("mod");
    let archive_bytes = fs::read(&archive).expect("archive bytes");
    let expected = inspect(&archive).expect("inspect").sha256;
    fixture
        .store
        .set_history_context(context(&fixture, "mod", "mod.zip"));
    let package = fixture
        .install_release(
            &archive,
            CatalogRelease::new(reference("mod"), Vec::new()).with_version("1.0".into()),
        )
        .expect("install");

    let operations = fixture.store.history(None).expect("history");
    assert_eq!(operations.len(), 1);
    let installed = &operations[0];
    assert_eq!(installed.kind, HistoryKind::Install);
    assert_eq!(installed.id, package.id);
    assert_eq!(installed.game_id, "test-game");
    assert_eq!(installed.installation, fixture.game);
    assert!(!installed.recorded_at.is_empty());
    let recorded = &installed.packages[0];
    assert_eq!(recorded.mod_id, "mod");
    assert_eq!(
        recorded.package_id_after.as_deref(),
        Some(package.id.as_str())
    );
    assert!(recorded.package_id_before.is_none());
    assert!(recorded.release_before.is_none());
    let after = recorded.release_after.as_ref().expect("after release");
    assert_eq!(after.version.as_deref(), Some("1.0"));
    assert!(matches!(
        after.source,
        Some(PackageSource::Manual { ref filename }) if filename == "mod.zip"
    ));
    assert_eq!(after.archive_sha256.as_deref(), Some(expected.as_str()));
    assert_eq!(
        after.archive_blake3.as_deref(),
        Some(format!("blake3:{}", blake3::hash(&archive_bytes).to_hex()).as_str())
    );
    assert_eq!(recorded.files.len(), 1);
    assert!(recorded.files[0].digest_before.is_none());
    assert!(recorded.files[0].digest_after.is_some());

    fixture
        .store
        .set_history_context(context(&fixture, "mod", "mod.zip"));
    fixture.uninstall(&package.id).expect("uninstall");
    let operations = fixture.store.history(None).expect("history");
    assert_eq!(operations.len(), 2);
    let removed = &operations[0];
    assert_eq!(removed.kind, HistoryKind::Uninstall);
    let removed_package = &removed.packages[0];
    assert_eq!(
        removed_package.package_id_before.as_deref(),
        Some(package.id.as_str())
    );
    assert!(removed_package.package_id_after.is_none());
    assert!(removed_package.release_after.is_none());
    assert_eq!(
        removed_package
            .release_before
            .as_ref()
            .and_then(|release| release.version.as_deref()),
        Some("1.0")
    );
    assert!(removed_package.files[0].digest_before.is_some());
    assert!(removed_package.files[0].digest_after.is_none());
}

#[test]
fn update_records_both_releases_and_both_file_digests() {
    let mut fixture = Fixture::new();
    let directory = fixture.state_path().parent().expect("state").to_path_buf();
    let v1 = archive_with(&directory, "v1", b"one");
    let v2 = archive_with(&directory, "v2", b"two");
    fixture
        .store
        .set_history_context(context(&fixture, "base", "v1.zip"));
    let release_v1 = CatalogRelease::new(reference("base"), Vec::new()).with_version("1.0".into());
    let package = fixture
        .install_release(&v1, release_v1)
        .expect("install v1");
    let before_digest = sha256(b"one");
    let after_digest = sha256(b"two");

    fixture
        .store
        .set_history_context(context(&fixture, "base", "v2.zip"));
    let release_v2 = CatalogRelease::new(reference("base"), Vec::new()).with_version("2.0".into());
    let plan_v2 = plan(&fixture.game, &v2);
    let updated = fixture
        .store
        .update_inner(
            &fixture.game,
            &package.id,
            &v2,
            &plan_v2,
            &release_v2,
            &[],
            NativeInjectorPolicy::Refuse,
            None,
        )
        .expect("update");

    let operations = fixture.store.history(None).expect("history");
    assert_eq!(operations.len(), 2);
    let operation = &operations[0];
    assert_eq!(operation.kind, HistoryKind::Update);
    let package_record = &operation.packages[0];
    assert_eq!(
        package_record
            .release_before
            .as_ref()
            .and_then(|release| release.version.as_deref()),
        Some("1.0")
    );
    assert_eq!(
        package_record
            .release_after
            .as_ref()
            .and_then(|release| release.version.as_deref()),
        Some("2.0")
    );
    assert_eq!(
        package_record.package_id_before.as_deref(),
        Some(package.id.as_str())
    );
    assert_eq!(
        package_record.package_id_after.as_deref(),
        Some(updated.id.as_str())
    );
    assert_eq!(package_record.files.len(), 1);
    assert_eq!(
        package_record.files[0].digest_before.as_deref(),
        Some(before_digest.as_str())
    );
    assert_eq!(
        package_record.files[0].digest_after.as_deref(),
        Some(after_digest.as_str())
    );
}

#[test]
fn cancelled_or_rolled_back_operations_write_no_history() {
    let mut fixture = Fixture::new();
    let archive = fixture.archive("mod");
    fixture
        .store
        .set_history_context(context(&fixture, "mod", "mod.zip"));
    let token = CancellationToken::new();
    token.cancel();
    fixture
        .store
        .set_control(OperationControl::new(token, |_| {}));
    let release = || CatalogRelease::new(reference("mod"), Vec::new());
    assert!(fixture.install_release(&archive, release()).is_err());
    assert!(fixture.store.history(None).expect("history").is_empty());
    assert!(fixture
        .store
        .installed(&fixture.game)
        .expect("installed")
        .is_empty());

    fixture.store.set_control(OperationControl::default());
    fixture
        .store
        .set_history_context(context(&fixture, "mod", "mod.zip"));
    let proposal = plan(&fixture.game, &archive);
    assert!(fixture
        .store
        .install_operations_inner(&fixture.game, &archive, &proposal, Some(1))
        .is_err());
    assert!(fixture.store.history(None).expect("history").is_empty());
    assert!(fixture
        .store
        .installed(&fixture.game)
        .expect("installed")
        .is_empty());
}

#[test]
fn crash_before_the_commit_point_writes_no_history_after_recovery() {
    let mut fixture = Fixture::new();
    let archive = fixture.archive("mod");
    fixture
        .store
        .set_history_context(context(&fixture, "mod", "mod.zip"));
    let proposal = plan(&fixture.game, &archive);
    let release = CatalogRelease::new(reference("mod"), Vec::new());
    let result = super::catalog::CatalogInstall {
        release: &release,
        replacing: None,
    };
    assert!(fixture
        .store
        .install_locked(
            &fixture.game,
            &archive,
            &proposal,
            Some(Fault::CrashAfter(1)),
            &[],
            Some(result),
            NativeInjectorPolicy::Refuse,
            None,
        )
        .is_err());
    fixture.store.recover().expect("recover");
    assert!(fixture.store.history(None).expect("history").is_empty());
    assert!(fixture
        .store
        .installed(&fixture.game)
        .expect("installed")
        .is_empty());
}

#[test]
fn a_failed_history_insert_rolls_back_ownership() {
    let mut fixture = Fixture::new();
    fixture
        .store
        .connection
        .execute_batch(
            "CREATE TRIGGER fail_history BEFORE INSERT ON history_operations
             BEGIN SELECT RAISE(ABORT, 'injected history failure'); END;",
        )
        .expect("trigger");
    let archive = fixture.archive("mod");
    fixture
        .store
        .set_history_context(context(&fixture, "mod", "mod.zip"));
    let result = fixture.install_release(
        &archive,
        CatalogRelease::new(reference("mod"), Vec::new()).with_version("1.0".into()),
    );
    assert!(result.is_err());
    assert!(fixture
        .store
        .installed(&fixture.game)
        .expect("installed")
        .is_empty());
    assert!(fixture.store.history(None).expect("history").is_empty());
    assert!(fixture.store.pending().expect("pending").is_empty());
    assert!(!fixture.game.join("addons/mod.plugin").exists());
}

#[test]
fn a_successful_stack_records_one_operation_with_every_package() {
    let mut fixture = Fixture::new();
    let first = fixture.archive("first");
    let second = fixture.archive("second");
    let members = [
        StackMember::new(
            first,
            CatalogRelease::new(reference("first"), Vec::new()).with_version("1.0".into()),
            Vec::new(),
        ),
        StackMember::new(
            second,
            CatalogRelease::new(reference("second"), Vec::new()).with_version("1.0".into()),
            Vec::new(),
        ),
    ];
    let context = HistoryContext::new("test-game", fixture.game.clone())
        .with_source(&reference("first"), source("first.zip"))
        .with_source(&reference("second"), source("second.zip"));
    fixture.store.set_history_context(context);
    let game = fixture.game.clone();
    let installed = fixture
        .store
        .install_catalog_stack(&game, &members, NativeInjectorPolicy::Refuse, |member| {
            Ok(plan(&game, &member.archive))
        })
        .expect("stack");
    assert_eq!(installed.len(), 2);
    let operations = fixture.store.history(None).expect("history");
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0].kind, HistoryKind::Install);
    assert_eq!(operations[0].packages.len(), 2);
    let mut ids: Vec<&str> = operations[0]
        .packages
        .iter()
        .filter_map(|package| package.package_id_after.as_deref())
        .collect();
    ids.sort_unstable();
    let mut expected: Vec<&str> = installed
        .iter()
        .map(|package| package.id.as_str())
        .collect();
    expected.sort_unstable();
    assert_eq!(ids, expected);
    assert!(operations[0].packages.iter().all(|package| package
        .release_after
        .as_ref()
        .and_then(|release| release.version.as_deref())
        == Some("1.0")));
}

#[test]
fn a_failed_stack_rollback_writes_no_history() {
    let mut fixture = Fixture::new();
    let first = fixture.archive("first");
    let second = fixture.archive("second");
    let members = [
        StackMember::new(
            first.clone(),
            CatalogRelease::new(reference("first"), Vec::new()).with_version("1.0".into()),
            Vec::new(),
        ),
        StackMember::new(
            second,
            CatalogRelease::new(reference("second"), Vec::new()).with_version("1.0".into()),
            Vec::new(),
        ),
    ];
    fixture
        .store
        .set_history_context(context(&fixture, "first", "first.zip"));
    let game = fixture.game.clone();
    let result = fixture.store.install_catalog_stack(
        &game,
        &members,
        NativeInjectorPolicy::Refuse,
        |member| {
            if member.release.reference.mod_id == "first" {
                Ok(plan(&game, &member.archive))
            } else {
                anyhow::bail!("injected stack failure")
            }
        },
    );
    assert!(result.is_err());
    assert!(fixture.store.history(None).expect("history").is_empty());
    assert!(fixture
        .store
        .installed(&game)
        .expect("installed")
        .is_empty());
}

#[test]
fn history_lists_newest_first_and_filters_by_game() {
    let mut fixture = Fixture::new();
    let first = fixture.archive("first");
    let second = fixture.archive("second");
    fixture
        .store
        .set_history_context(context(&fixture, "first", "first.zip"));
    let first_package = fixture
        .install_release(
            &first,
            CatalogRelease::new(reference("first"), Vec::new()).with_version("1.0".into()),
        )
        .expect("first install");
    fixture
        .store
        .set_history_context(context(&fixture, "second", "second.zip"));
    let second_package = fixture
        .install_release(
            &second,
            CatalogRelease::new(reference("second"), Vec::new()).with_version("1.0".into()),
        )
        .expect("second install");

    let all = fixture.store.history(None).expect("history");
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].id, second_package.id);
    assert_eq!(all[1].id, first_package.id);
    assert_eq!(
        fixture
            .store
            .history(Some("test-game"))
            .expect("filtered")
            .len(),
        2
    );
    assert!(fixture
        .store
        .history(Some("other-game"))
        .expect("filtered")
        .is_empty());
}

#[test]
fn overlapping_writes_each_report_the_record_they_committed() {
    let mut fixture = Fixture::new();
    let first_archive = fixture.archive("first");
    let second_archive = fixture.archive("second");
    let game = fixture.game.clone();

    fixture
        .store
        .set_history_context(context(&fixture, "first", "first.zip"));
    let first = with_module_source(Some(&module_source()), || {
        fixture.store.install_catalog_release(
            &game,
            &first_archive,
            &plan(&game, &first_archive),
            &CatalogRelease::new(reference("first"), Vec::new()),
            NativeInjectorPolicy::Refuse,
        )
    })
    .expect("first install");

    // A second write commits before the first caller reads its result. An ID
    // inferred from history would now see both records and lose the first.
    let mut second_store = Store::open(&fixture.state_path()).expect("second store");
    second_store.set_history_context(context(&fixture, "second", "second.zip"));
    let second = with_module_source(Some(&module_source()), || {
        second_store.install_catalog_release(
            &game,
            &second_archive,
            &plan(&game, &second_archive),
            &CatalogRelease::new(reference("second"), Vec::new()),
            NativeInjectorPolicy::Refuse,
        )
    })
    .expect("second install");

    assert_eq!(
        fixture.store.committed_operation_id(),
        Some(first.id.as_str())
    );
    assert_eq!(
        second_store.committed_operation_id(),
        Some(second.id.as_str())
    );
    assert_ne!(first.id, second.id);

    let operations = fixture.store.history(None).expect("history");
    assert_eq!(operations.len(), 2);
    assert_eq!(operations[0].id, second.id);
    assert_eq!(operations[1].id, first.id);
    assert_eq!(operations[0].packages[0].mod_id, "second");
    assert_eq!(operations[1].packages[0].mod_id, "first");
}
