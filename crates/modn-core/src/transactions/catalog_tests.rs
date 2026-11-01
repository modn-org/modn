//! Catalog provenance, lock-held dependency readiness and dependent protection.
use super::{CatalogDependency, CatalogReference, CatalogRelease, ManagedPackage};
use crate::archive::inspect;
use crate::game_modules::with_module_source;
use crate::planning::{InstallPlan, PlannedFile};
use crate::state::Store;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};
use zip::write::SimpleFileOptions;

struct Fixture {
    dir: TempDir,
    store: Store,
    game: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempdir().unwrap();
        let game = dir.path().join("game");
        fs::create_dir_all(game.join("bin")).unwrap();
        fs::write(game.join("bin/TestGame.exe"), b"game-exe").unwrap();
        let game = game.canonicalize().unwrap();
        let store = Store::open(&dir.path().join("state/state.sqlite")).unwrap();
        Self { dir, store, game }
    }

    fn archive(&self, name: &str) -> PathBuf {
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
        let release = CatalogRelease {
            reference: reference(mod_id),
            dependencies,
        };
        let proposal = plan(&self.game, archive);
        let game = self.game.clone();
        with_module_source(Some(&module_source()), || {
            self.store
                .install_catalog_release(&game, archive, &proposal, &release)
        })
    }

    fn uninstall(&mut self, package_id: &str) -> anyhow::Result<()> {
        let game = self.game.clone();
        with_module_source(Some(&module_source()), || {
            self.store.uninstall(&game, package_id)
        })
    }

    fn count(&self, table: &str) -> i64 {
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

fn reference(mod_id: &str) -> CatalogReference {
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
