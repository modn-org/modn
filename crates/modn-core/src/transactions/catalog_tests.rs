//! Catalog provenance, lock-held dependency readiness and dependent protection.
use super::catalog::CatalogInstall;
use super::{
    CatalogDependency, CatalogReference, CatalogRelease, DependencyState, ManagedPackage,
    StackMember,
};
use crate::archive::inspect;
use crate::game_modules::with_module_source;
use crate::planning::{InstallPlan, PlannedFile};
use crate::state::Store;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};
use zip::write::SimpleFileOptions;

pub(super) struct Fixture {
    dir: TempDir,
    pub(super) store: Store,
    pub(super) game: PathBuf,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let dir = tempdir().unwrap();
        let game = dir.path().join("game");
        fs::create_dir_all(game.join("bin")).unwrap();
        fs::write(game.join("bin/TestGame.exe"), b"game-exe").unwrap();
        let game = game.canonicalize().unwrap();
        let store = Store::open(&dir.path().join("state/state.sqlite")).unwrap();
        Self { dir, store, game }
    }

    pub(super) fn archive(&self, name: &str) -> PathBuf {
        let path = self.dir.path().join(format!("{name}.zip"));
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        zip.start_file(format!("pack/{name}.plugin"), SimpleFileOptions::default())
            .unwrap();
        zip.write_all(name.as_bytes()).unwrap();
        zip.finish().unwrap();
        path
    }

    fn install(
        &mut self,
        archive: &Path,
        mod_id: &str,
        dependencies: Vec<CatalogDependency>,
    ) -> anyhow::Result<ManagedPackage> {
        self.install_release(
            archive,
            CatalogRelease::new(reference(mod_id), dependencies),
        )
    }

    pub(super) fn install_release(
        &mut self,
        archive: &Path,
        release: CatalogRelease,
    ) -> anyhow::Result<ManagedPackage> {
        let proposal = plan(&self.game, archive);
        let game = self.game.clone();
        with_module_source(Some(&module_source()), || {
            self.store
                .install_catalog_release(&game, archive, &proposal, &release)
        })
    }

    pub(super) fn uninstall(&mut self, package_id: &str) -> anyhow::Result<()> {
        let game = self.game.clone();
        with_module_source(Some(&module_source()), || {
            self.store.uninstall(&game, package_id)
        })
    }

    pub(super) fn count(&self, table: &str) -> i64 {
        self.store
            .connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    }
}

fn module_source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/test-game")
}

pub(super) fn reference(mod_id: &str) -> CatalogReference {
    CatalogReference {
        bucket: "deps".into(),
        game_id: "test-game".into(),
        mod_id: mod_id.into(),
    }
}

fn dependency_on(archive: &Path, mod_id: &str) -> CatalogDependency {
    CatalogDependency {
        reference: reference(mod_id),
        archive_sha256: inspect(archive).unwrap().sha256,
    }
}

fn plan(game: &Path, archive: &Path) -> InstallPlan {
    let checked = inspect(archive).unwrap();
    InstallPlan {
        archive_sha256: checked.sha256,
        game_installation: game.to_path_buf(),
        status: crate::planning::PlanStatus::Reviewable,
        metadata_only: true,
        operations: checked
            .files
            .into_iter()
            .map(|entry| PlannedFile {
                destination: format!("addons/{}", entry.entry.rsplit('/').next().unwrap()),
                source: entry.entry,
                bytes: entry.bytes,
                kind: "file-copy".into(),
                owned_directory: None,
            })
            .collect(),
        blockers: Vec::new(),
        skipped_generated_files: Vec::new(),
    }
}

#[test]
fn recorded_dependency_blocks_uninstall_until_dependent_is_removed() {
    let mut fixture = Fixture::new();
    let base_archive = fixture.archive("base");
    let dependent_archive = fixture.archive("dependent");
    let base = fixture.install(&base_archive, "base", Vec::new()).unwrap();
    let dependent = fixture
        .install(
            &dependent_archive,
            "dependent",
            vec![dependency_on(&base_archive, "base")],
        )
        .unwrap();
    assert_eq!(fixture.count("catalog_provenance"), 2);
    let edge: (String, String) = fixture
        .store
        .connection
        .query_row(
            "SELECT package_id, dependency_id FROM package_dependencies",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(edge, (dependent.id.clone(), base.id.clone()));

    let blocked = fixture.uninstall(&base.id).unwrap_err().to_string();
    assert!(blocked.contains("required by"), "{blocked}");
    assert!(blocked.contains("deps/test-game/dependent"), "{blocked}");
    assert!(fixture.game.join("addons/base.plugin").is_file());
    assert_eq!(fixture.store.installed(&fixture.game).unwrap().len(), 2);

    fixture.uninstall(&dependent.id).unwrap();
    assert_eq!(fixture.count("package_dependencies"), 0);
    fixture.uninstall(&base.id).unwrap();
    assert_eq!(fixture.count("catalog_provenance"), 0);
    assert!(!fixture.game.join("addons/base.plugin").exists());
}

#[test]
fn missing_or_drifted_dependency_blocks_install_without_writes() {
    let mut fixture = Fixture::new();
    let base_archive = fixture.archive("base");
    let dependent_archive = fixture.archive("dependent");
    let requirement = vec![dependency_on(&base_archive, "base")];

    let missing = fixture
        .install(&dependent_archive, "dependent", requirement.clone())
        .unwrap_err()
        .to_string();
    assert!(
        missing.contains("dependency not installed or intact: deps/test-game/base"),
        "{missing}"
    );

    fixture.install(&base_archive, "base", Vec::new()).unwrap();
    fs::write(fixture.game.join("addons/base.plugin"), b"modified").unwrap();
    let drifted = fixture
        .install(&dependent_archive, "dependent", requirement)
        .unwrap_err()
        .to_string();
    assert!(
        drifted.contains("dependency not installed or intact"),
        "{drifted}"
    );

    assert!(!fixture.game.join("addons/dependent.plugin").exists());
    assert_eq!(fixture.store.installed(&fixture.game).unwrap().len(), 1);
    assert_eq!(fixture.count("pending_transactions"), 0);
    assert_eq!(fixture.count("package_dependencies"), 0);
}

#[test]
fn plain_archive_installs_record_no_catalog_provenance() {
    let mut fixture = Fixture::new();
    let archive = fixture.archive("plain");
    let proposal = plan(&fixture.game, &archive);
    let game = fixture.game.clone();
    let package = with_module_source(Some(&module_source()), || {
        fixture
            .store
            .install_operations_zip(&game, &archive, &proposal)
    })
    .unwrap();
    assert_eq!(fixture.count("catalog_provenance"), 0);
    fixture.uninstall(&package.id).unwrap();
}

#[test]
fn dependency_readiness_needs_no_game_module() {
    let mut fixture = Fixture::new();
    let base_archive = fixture.archive("base");
    let base = fixture.install(&base_archive, "base", Vec::new()).unwrap();
    let requirement = vec![dependency_on(&base_archive, "base")];

    // No module source is selected here, so any Lua discovery would fail.
    let ready = fixture
        .store
        .ready_dependencies(&fixture.game, &requirement)
        .unwrap();
    assert_eq!(ready, vec![base.id]);

    fs::write(fixture.game.join("addons/base.plugin"), b"modified").unwrap();
    let drifted = fixture
        .store
        .ready_dependencies(&fixture.game, &requirement)
        .unwrap_err()
        .to_string();
    assert!(
        drifted.contains("dependency not installed or intact"),
        "{drifted}"
    );
}

#[test]
fn dependency_states_distinguish_ready_damaged_other_release_and_missing() {
    let mut fixture = Fixture::new();
    let base_archive = fixture.archive("base");
    let other_archive = fixture.archive("other");
    let pinned = dependency_on(&base_archive, "base");
    let absent = dependency_on(&fixture.archive("absent"), "absent");
    let states = |fixture: &Fixture| {
        fixture
            .store
            .dependency_states(&fixture.game, &[pinned.clone(), absent.clone()])
            .unwrap()
    };
    assert_eq!(
        states(&fixture),
        [DependencyState::Missing, DependencyState::Missing]
    );

    // A managed package recorded as catalog mod `base`, from another archive.
    let other = fixture.install(&other_archive, "base", Vec::new()).unwrap();
    assert_eq!(
        states(&fixture)[0],
        DependencyState::OtherRelease {
            package_ids: vec![other.id.clone()]
        }
    );
    fixture.uninstall(&other.id).unwrap();

    let base = fixture.install(&base_archive, "base", Vec::new()).unwrap();
    assert_eq!(
        states(&fixture),
        [
            DependencyState::Ready {
                package_ids: vec![base.id.clone()]
            },
            DependencyState::Missing
        ]
    );
    fs::write(fixture.game.join("addons/base.plugin"), b"modified").unwrap();
    assert_eq!(
        states(&fixture)[0],
        DependencyState::Damaged {
            package_ids: vec![base.id]
        }
    );
    assert_eq!(fixture.count("pending_transactions"), 0);
}

#[test]
fn read_only_state_without_provenance_table_reports_missing_dependencies() {
    let fixture = Fixture::new();
    let requirement = vec![dependency_on(&fixture.archive("base"), "base")];
    let path = fixture.dir.path().join("state/state.sqlite");
    // Simulate state written before catalog provenance existed.
    fixture
        .store
        .connection
        .execute_batch("DROP TABLE package_dependencies; DROP TABLE catalog_provenance;")
        .unwrap();
    let store = Store::open_readonly(&path).unwrap();

    assert_eq!(
        store
            .dependency_states(&fixture.game, &requirement)
            .unwrap(),
        [DependencyState::Missing]
    );
    let error = store
        .ready_dependencies(&fixture.game, &requirement)
        .unwrap_err()
        .to_string();
    assert_eq!(
        error,
        "dependency not installed or intact: deps/test-game/base; install or restore it first"
    );
}

fn member(archive: &Path, mod_id: &str, dependencies: Vec<CatalogDependency>) -> StackMember {
    StackMember::new(
        archive.to_path_buf(),
        CatalogRelease::new(reference(mod_id), dependencies),
        Vec::new(),
    )
}

impl Fixture {
    fn stack(
        &mut self,
        members: &[StackMember],
        mut fail_on: Option<&str>,
    ) -> anyhow::Result<Vec<ManagedPackage>> {
        let game = self.game.clone();
        with_module_source(Some(&module_source()), || {
            self.store.install_catalog_stack(&game, members, |member| {
                if fail_on == Some(member.release.reference.mod_id.as_str()) {
                    fail_on = None;
                    anyhow::bail!("injected plan failure");
                }
                Ok(plan(&game, &member.archive))
            })
        })
    }

    fn pending_stack_rows(&self) -> i64 {
        self.count("pending_stacks") + self.count("pending_stack_packages")
    }
}

#[test]
fn stack_installs_members_in_order_as_separate_packages() {
    let mut fixture = Fixture::new();
    let base = fixture.archive("base");
    let top = fixture.archive("top");
    let game = fixture.game.clone();
    let members = [
        member(&base, "base", Vec::new()),
        member(&top, "top", vec![dependency_on(&base, "base")]),
    ];
    let packages = with_module_source(Some(&module_source()), || {
        fixture
            .store
            .install_catalog_stack(&game, &members, |member| {
                // Each plan is made after the earlier members are installed.
                if member.release.reference.mod_id == "top" {
                    assert!(game.join("addons/base.plugin").is_file());
                }
                Ok(plan(&game, &member.archive))
            })
    })
    .unwrap();
    assert_eq!(packages.len(), 2);
    assert_eq!(fixture.store.installed(&fixture.game).unwrap().len(), 2);
    assert_eq!(fixture.count("catalog_provenance"), 2);
    assert_eq!(fixture.count("package_dependencies"), 1);
    assert_eq!(fixture.pending_stack_rows(), 0);
    // Recorded links still protect the dependency after the stack commits.
    let blocked = fixture.uninstall(&packages[0].id).unwrap_err().to_string();
    assert!(blocked.contains("required by"), "{blocked}");
}

#[test]
fn failed_member_rolls_back_every_earlier_member() {
    let mut fixture = Fixture::new();
    let base = fixture.archive("base");
    let middle = fixture.archive("middle");
    let top = fixture.archive("top");
    let members = [
        member(&base, "base", Vec::new()),
        member(&middle, "middle", vec![dependency_on(&base, "base")]),
        member(&top, "top", vec![dependency_on(&middle, "middle")]),
    ];
    let error = format!("{:#}", fixture.stack(&members, Some("top")).unwrap_err());
    assert!(error.contains("rolled back 2 package(s)"), "{error}");
    assert!(error.contains("injected plan failure"), "{error}");
    assert!(error.contains("deps/test-game/top"), "{error}");
    for name in ["base", "middle", "top"] {
        assert!(!fixture.game.join(format!("addons/{name}.plugin")).exists());
    }
    assert!(fixture.store.installed(&fixture.game).unwrap().is_empty());
    assert_eq!(fixture.count("catalog_provenance"), 0);
    assert_eq!(fixture.count("package_dependencies"), 0);
    assert_eq!(fixture.count("pending_transactions"), 0);
    assert_eq!(fixture.pending_stack_rows(), 0);

    // A member whose dependency is not ready also rolls the stack back.
    let unready = [
        member(&base, "base", Vec::new()),
        member(&top, "top", vec![dependency_on(&middle, "middle")]),
    ];
    let error = format!("{:#}", fixture.stack(&unready, None).unwrap_err());
    assert!(
        error.contains("dependency not installed or intact"),
        "{error}"
    );
    assert!(fixture.store.installed(&fixture.game).unwrap().is_empty());
    assert_eq!(fixture.pending_stack_rows(), 0);
}

#[test]
fn interrupted_stack_blocks_writes_until_recovery_rolls_it_back() {
    let mut fixture = Fixture::new();
    let base = fixture.archive("base");
    let other = fixture.archive("other");
    let game = fixture.game.clone();
    let release = CatalogRelease::new(reference("base"), Vec::new());
    // Simulate a crash after the first member committed, before the stack did.
    let proposal = plan(&game, &base);
    let stack_id = fixture.store.begin_stack(&game).unwrap();
    with_module_source(Some(&module_source()), || {
        fixture.store.install_locked(
            &game,
            &base,
            &proposal,
            None,
            &[],
            Some(CatalogInstall {
                release: &release,
                stack_id: Some(&stack_id),
            }),
        )
    })
    .unwrap();
    assert!(game.join("addons/base.plugin").is_file());
    assert_eq!(fixture.count("pending_stack_packages"), 1);

    let blocked = fixture
        .install(&other, "other", Vec::new())
        .unwrap_err()
        .to_string();
    assert!(
        blocked.contains("unfinished installation stack"),
        "{blocked}"
    );
    let package = fixture.store.installed(&game).unwrap().remove(0);
    let blocked = fixture.uninstall(&package.id).unwrap_err().to_string();
    assert!(
        blocked.contains("unfinished installation stack"),
        "{blocked}"
    );

    let recovered = with_module_source(Some(&module_source()), || fixture.store.recover()).unwrap();
    assert_eq!(recovered, 1);
    assert!(!game.join("addons/base.plugin").exists());
    assert!(fixture.store.installed(&game).unwrap().is_empty());
    assert_eq!(fixture.count("catalog_provenance"), 0);
    assert_eq!(fixture.pending_stack_rows(), 0);
    fixture.install(&other, "other", Vec::new()).unwrap();
}
