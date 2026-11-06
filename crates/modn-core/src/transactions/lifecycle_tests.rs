use super::install::Fault;
use super::{AdoptionApproval, CleanupApproval};
use crate::planning::{preview_adoption, preview_install_zip, PlanStatus};
use crate::state::Store;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};
use zip::write::SimpleFileOptions;

mod adoption;
mod content_prune;
mod content_root;

struct Fixture {
    root: TempDir,
    store: Store,
    game: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempdir().unwrap();
        let game = root.path().join("game");
        fs::create_dir_all(game.join("bin/x64")).unwrap();
        fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"game").unwrap();
        let store = Store::open(&root.path().join("state/state.sqlite")).unwrap();
        Self { root, store, game }
    }

    fn archive(&self, name: &str, entries: &[(&str, &[u8])]) -> PathBuf {
        let path = self.root.path().join(name);
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        for (name, bytes) in entries {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    fn write(&self, relative: &str, bytes: &[u8]) {
        let path = self.game.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn install(&mut self, archive: &Path) -> super::ManagedPackage {
        let plan = preview_install_zip("cyberpunk-2077", &self.game, archive).unwrap();
        assert_eq!(plan.status, PlanStatus::Reviewable, "{:?}", plan.blockers);
        self.store
            .install_operations_zip(&self.game, archive, &plan)
            .unwrap()
    }

    fn adopt(
        &mut self,
        archive: &Path,
        approval: AdoptionApproval,
    ) -> anyhow::Result<super::ManagedPackage> {
        let plan = preview_adoption("cyberpunk-2077", &self.game, archive)?;
        self.store.adopt_archive(
            "cyberpunk-2077",
            &self.game,
            archive,
            &plan.archive_sha256,
            approval,
        )
    }
}

#[test]
fn redmod_lifecycle_reports_individual_files_as_managed() {
    crate::test_support::seeded(|| {
        let mut fixture = Fixture::new();
        let archive = fixture.archive(
            "redmod.zip",
            &[
                (
                    "Wrapper/mods/My Mod/info.json",
                    br#"{"name":"My Mod","version":"1.0"}"#,
                ),
                ("Wrapper/mods/My Mod/archives/content.archive", b"content"),
            ],
        );
        let package = fixture.install(&archive);
        assert_eq!(package.files.len(), 2);
        assert_eq!(
            fs::read(fixture.game.join("mods/My Mod/archives/content.archive")).unwrap(),
            b"content"
        );
        let report = crate::inventory::scan_with_ownership_for(
            &fixture.store,
            "cyberpunk-2077",
            &fixture.game,
        )
        .unwrap();
        assert_eq!(report.owned_files.len(), 2);
        assert!(report
            .owned_files
            .iter()
            .all(|item| item.package_id == package.id
                && item.status == crate::scan::IntegrityStatus::Intact));
        assert_eq!(report.mods.len(), 1);
        assert!(!report.mods[0].managed);
        let doctor = crate::inventory::doctor_with_ownership_for(
            &fixture.store,
            "cyberpunk-2077",
            &fixture.game,
        )
        .unwrap();
        assert_eq!(doctor.scan.owned_files.len(), 2);
        fixture.store.uninstall(&fixture.game, &package.id).unwrap();
        assert!(!fixture.game.join("mods/My Mod").exists());
        assert!(fixture.store.installed(&fixture.game).unwrap().is_empty());
    });
}

#[test]
fn redmod_invalid_mixed_ambiguous_and_occupied_layouts_refuse_without_writes() {
    crate::test_support::seeded(|| {
        let mut fixture = Fixture::new();
        let cases: Vec<Vec<(&str, &[u8])>> = vec![
            vec![("mods/Example/archives/a.archive", b"a")],
            vec![("mods/Example/info.json", b"")],
            vec![("Mods/Example/info.json", b"{}")],
            vec![("mods/Example/INFO.JSON", b"{}")],
            vec![
                ("mods/Example/info.json", b"{}"),
                ("archive/pc/mod/a.archive", b"a"),
            ],
            vec![
                ("mods/Example/info.json", b"{}"),
                ("mods/Other/info.json", b"{}"),
            ],
            vec![
                ("mods/Example/info.json", b"{}"),
                ("mods/Example/unknown.bin", b"a"),
            ],
        ];
        for entries in cases {
            let archive = fixture.archive("invalid.zip", &entries);
            let plan = preview_install_zip("cyberpunk-2077", &fixture.game, &archive).unwrap();
            assert_eq!(plan.status, PlanStatus::Blocked, "{entries:?}");
            assert!(fixture
                .store
                .install_operations_zip(&fixture.game, &archive, &plan)
                .is_err());
            assert!(!fixture.game.join("mods").exists());
            assert!(fixture.store.installed(&fixture.game).unwrap().is_empty());
        }
        fixture.write("mods/Example/unmanaged.bin", b"keep");
        let archive = fixture.archive("occupied.zip", &[("mods/Example/info.json", b"{}")]);
        let plan = preview_install_zip("cyberpunk-2077", &fixture.game, &archive).unwrap();
        assert_eq!(plan.status, PlanStatus::Blocked);
        assert!(fixture
            .store
            .install_operations_zip(&fixture.game, &archive, &plan)
            .is_err());
        assert_eq!(
            fs::read(fixture.game.join("mods/Example/unmanaged.bin")).unwrap(),
            b"keep"
        );
        eprintln!("REDmod refusal: 8 layouts checked");
    });
}

#[test]
fn adoption_requires_matching_files_and_acknowledgement_without_directory_claims() {
    crate::test_support::seeded(|| {
        let mut fixture = Fixture::new();
        let a = "archive/pc/mod/a.archive";
        let b = "archive/pc/mod/b.archive";
        let archive = fixture.archive("manual.zip", &[(a, b"one"), (b, b"two")]);
        fixture.write(a, b"one");
        fixture.write(b, b"bad");
        fixture.write("archive/pc/mod/unmanaged.archive", b"keep");
        assert!(fixture.adopt(&archive, AdoptionApproval::Denied).is_err());
        assert!(fixture
            .adopt(&archive, AdoptionApproval::Acknowledged)
            .is_err());
        assert!(fixture.store.installed(&fixture.game).unwrap().is_empty());
        fixture.write(b, b"two");
        let before = fs::metadata(fixture.game.join(a))
            .unwrap()
            .modified()
            .unwrap();
        let package = fixture
            .adopt(&archive, AdoptionApproval::Acknowledged)
            .unwrap();
        assert_eq!(package.files.len(), 2);
        assert!(package.directories.is_empty());
        assert_eq!(
            before,
            fs::metadata(fixture.game.join(a))
                .unwrap()
                .modified()
                .unwrap()
        );
        assert_eq!(fs::read(fixture.game.join(b)).unwrap(), b"two");
        assert!(fixture
            .adopt(&archive, AdoptionApproval::Acknowledged)
            .is_err());
        assert_eq!(fixture.store.installed(&fixture.game).unwrap().len(), 1);
        fixture.store.uninstall(&fixture.game, &package.id).unwrap();
        assert_eq!(
            fs::read(fixture.game.join("archive/pc/mod/unmanaged.archive")).unwrap(),
            b"keep"
        );
        assert!(fixture.store.pending().unwrap().is_empty());
    });
}

#[test]
fn adoption_database_failure_rolls_back_all_claims_and_archive_provenance() {
    crate::test_support::seeded(|| {
        let mut fixture = Fixture::new();
        let archive = fixture.archive(
            "manual.zip",
            &[
                ("archive/pc/mod/a.archive", b"one"),
                ("archive/pc/mod/b.archive", b"two"),
            ],
        );
        fixture.write("archive/pc/mod/a.archive", b"one");
        fixture.write("archive/pc/mod/b.archive", b"two");
        fixture.store.connection.execute_batch("CREATE TRIGGER reject_second BEFORE INSERT ON managed_files WHEN NEW.relative_path LIKE '%b.archive' BEGIN SELECT RAISE(ABORT, 'injected failure'); END;").unwrap();
        assert!(fixture
            .adopt(&archive, AdoptionApproval::Acknowledged)
            .is_err());
        assert!(fixture.store.installed(&fixture.game).unwrap().is_empty());
        let count: i64 = fixture
            .store
            .connection
            .query_row("SELECT COUNT(*) FROM managed_files", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(
            fs::read(fixture.game.join("archive/pc/mod/a.archive")).unwrap(),
            b"one"
        );
        assert_eq!(
            fs::read(fixture.game.join("archive/pc/mod/b.archive")).unwrap(),
            b"two"
        );
    });
}

#[test]
fn redmod_invalid_json_never_reaches_the_game_or_ownership() {
    crate::test_support::seeded(|| {
        for bytes in [b"not JSON".as_slice(), b"[]", b"null", b"{\"broken\":}"] {
            let mut fixture = Fixture::new();
            let archive = fixture.archive("invalid-json.zip", &[("mods/Example/info.json", bytes)]);
            let plan = preview_install_zip("cyberpunk-2077", &fixture.game, &archive).unwrap();
            assert_eq!(plan.status, PlanStatus::Reviewable);
            assert!(fixture
                .store
                .install_operations_zip(&fixture.game, &archive, &plan)
                .is_err());
            assert!(!fixture.game.join("mods").exists());
            assert!(fixture.store.installed(&fixture.game).unwrap().is_empty());
            assert!(fixture.store.pending().unwrap().is_empty());
            fixture.write("mods/Example/info.json", bytes);
            assert!(fixture
                .adopt(&archive, AdoptionApproval::Acknowledged)
                .is_err());
            assert_eq!(
                fs::read(fixture.game.join("mods/Example/info.json")).unwrap(),
                bytes
            );
        }
        eprintln!("REDmod JSON: 4 invalid contents rejected by install and adoption");
    });
}

#[test]
fn adoption_refuses_a_changed_reviewed_archive() {
    crate::test_support::seeded(|| {
        let mut fixture = Fixture::new();
        let path = "archive/pc/mod/manual.archive";
        let archive = fixture.archive("manual.zip", &[(path, b"one")]);
        fixture.write(path, b"one");
        let preview = preview_adoption("cyberpunk-2077", &fixture.game, &archive).unwrap();
        fixture.archive("manual.zip", &[(path, b"two")]);
        assert!(fixture
            .store
            .adopt_archive(
                "cyberpunk-2077",
                &fixture.game,
                &archive,
                &preview.archive_sha256,
                AdoptionApproval::Acknowledged
            )
            .is_err());
        assert!(fixture.store.installed(&fixture.game).unwrap().is_empty());
        assert_eq!(fs::read(fixture.game.join(path)).unwrap(), b"one");
    });
}

#[test]
fn cleanup_and_recovery_refuse_replaced_exclusive_directory_identity() {
    crate::test_support::seeded(|| {
        for interrupted in [false, true] {
            let mut fixture = Fixture::new();
            fixture.write("engine/tools/scc.exe", b"compiler");
            fs::create_dir_all(fixture.game.join("r6/scripts")).unwrap();
            let archive = fixture.archive(
                "owned.zip",
                &[
                    ("r6/scripts/Example/a.reds", b"one"),
                    ("r6/scripts/Example/b.reds", b"two"),
                ],
            );
            let package = fixture.install(&archive);
            fixture.write("r6/scripts/Example/a.reds", b"modified");
            if interrupted {
                assert!(fixture
                    .store
                    .cleanup_owned_inner(
                        &fixture.game,
                        &package.id,
                        CleanupApproval::Approved,
                        Some(Fault::CrashAfter(1))
                    )
                    .is_err());
            }
            let original = fixture.game.join("r6/scripts/Example");
            let saved = fixture.game.join("r6/scripts/Saved");
            fs::rename(&original, &saved).unwrap();
            fixture.write("r6/scripts/Example/a.reds", b"replacement");
            fixture.write("r6/scripts/Example/b.reds", b"replacement");
            let result = if interrupted {
                fixture.store.recover().map(|_| ())
            } else {
                fixture
                    .store
                    .cleanup_owned(&fixture.game, &package.id, CleanupApproval::Approved)
            };
            assert!(result.is_err());
            assert_eq!(fs::read(original.join("a.reds")).unwrap(), b"replacement");
            assert_eq!(fixture.store.installed(&fixture.game).unwrap().len(), 1);
            fs::remove_file(original.join("a.reds")).unwrap();
            fs::remove_file(original.join("b.reds")).unwrap();
            fs::remove_dir(&original).unwrap();
            fs::rename(&saved, &original).unwrap();
            fixture.store.recover().unwrap();
            assert_eq!(fs::read(original.join("a.reds")).unwrap(), b"modified");
        }
    });
}

#[test]
fn forced_cleanup_preserves_unknown_descendants_other_packages_and_recovery_bytes() {
    crate::test_support::seeded(|| {
        for crash in [false, true] {
            let mut fixture = Fixture::new();
            fixture.write("engine/tools/scc.exe", b"compiler");
            fs::create_dir_all(fixture.game.join("r6/scripts")).unwrap();
            let archive = fixture.archive(
                "owned.zip",
                &[
                    ("r6/scripts/Example/a.reds", b"one"),
                    ("r6/scripts/Example/b.reds", b"two"),
                ],
            );
            let package = fixture.install(&archive);
            assert_eq!(package.directories, vec!["r6/scripts/Example"]);
            let other_archive =
                fixture.archive("other.zip", &[("archive/pc/mod/other.archive", b"other")]);
            let other = fixture.install(&other_archive);
            fixture.write("r6/scripts/Example/a.reds", b"modified one");
            fixture.write("r6/scripts/Example/b.reds", b"modified two");
            fixture.write("r6/scripts/Example/unmanaged.txt", b"keep");
            assert!(fixture
                .store
                .cleanup_owned(&fixture.game, &package.id, CleanupApproval::Denied)
                .is_err());
            let fault = if crash {
                Fault::CrashAfter(1)
            } else {
                Fault::FailAfter(1)
            };
            assert!(fixture
                .store
                .cleanup_owned_inner(
                    &fixture.game,
                    &package.id,
                    CleanupApproval::Approved,
                    Some(fault)
                )
                .is_err());
            assert_eq!(fixture.store.pending().unwrap().len(), usize::from(crash));
            fixture.store.recover().unwrap();
            assert_eq!(
                fs::read(fixture.game.join("r6/scripts/Example/a.reds")).unwrap(),
                b"modified one"
            );
            assert_eq!(
                fs::read(fixture.game.join("r6/scripts/Example/b.reds")).unwrap(),
                b"modified two"
            );
            assert_eq!(fixture.store.installed(&fixture.game).unwrap().len(), 2);
            fixture
                .store
                .cleanup_owned(&fixture.game, &package.id, CleanupApproval::Approved)
                .unwrap();
            assert!(!fixture.game.join("r6/scripts/Example/a.reds").exists());
            assert!(!fixture.game.join("r6/scripts/Example/b.reds").exists());
            assert_eq!(
                fs::read(fixture.game.join("r6/scripts/Example/unmanaged.txt")).unwrap(),
                b"keep"
            );
            assert_eq!(
                fs::read(fixture.game.join("archive/pc/mod/other.archive")).unwrap(),
                b"other"
            );
            assert_eq!(
                fixture.store.installed(&fixture.game).unwrap()[0].id,
                other.id
            );
        }
        eprintln!("forced cleanup: automatic rollback and crash recovery checked");
    });
}

#[test]
fn lifecycle_authority_properties() {
    use proptest::prelude::*;
    use proptest::test_runner::{Config, RngSeed, TestRunner};
    use std::cell::Cell;
    crate::test_support::seeded(|| {
        let checked = Cell::new(0);
        let config = Config {
            cases: 32,
            rng_seed: RngSeed::Fixed(0x6060),
            failure_persistence: None,
            ..Config::default()
        };
        TestRunner::new(config)
            .run(
                &(
                    proptest::collection::vec(any::<u8>(), 1..128),
                    any::<bool>(),
                ),
                |(bytes, approve)| {
                    let mut fixture = Fixture::new();
                    let path = "archive/pc/mod/manual.archive";
                    let archive = fixture.archive("manual.zip", &[(path, &bytes)]);
                    fixture.write(path, &bytes);
                    fixture.write("archive/pc/mod/unmanaged.archive", b"unmanaged");
                    let approval = if approve {
                        AdoptionApproval::Acknowledged
                    } else {
                        AdoptionApproval::Denied
                    };
                    let result = fixture.adopt(&archive, approval);
                    prop_assert_eq!(result.is_ok(), approve);
                    prop_assert_eq!(fs::read(fixture.game.join(path)).unwrap(), bytes.clone());
                    if let Ok(package) = result {
                        let mut modified = bytes.clone();
                        modified.push(0xff);
                        fixture.write(path, &modified);
                        prop_assert!(fixture.store.uninstall(&fixture.game, &package.id).is_err());
                        prop_assert!(fixture
                            .store
                            .cleanup_owned(&fixture.game, &package.id, CleanupApproval::Denied)
                            .is_err());
                        prop_assert_eq!(fs::read(fixture.game.join(path)).unwrap(), modified);
                        fixture
                            .store
                            .cleanup_owned(&fixture.game, &package.id, CleanupApproval::Approved)
                            .unwrap();
                        prop_assert!(!fixture.game.join(path).exists());
                    }
                    prop_assert_eq!(
                        fs::read(fixture.game.join("archive/pc/mod/unmanaged.archive")).unwrap(),
                        b"unmanaged"
                    );
                    checked.set(checked.get() + 1);
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(checked.get(), 32);
        eprintln!(
            "lifecycle authority: {} generated cases checked",
            checked.get()
        );
    });
}
