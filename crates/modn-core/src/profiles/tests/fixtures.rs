use crate::archive::inspect;
use crate::planning::{InstallPlan, PlanStatus, PlannedFile};
use crate::profiles::{list, Profile};
use crate::state::Store;
use crate::transactions::{CatalogReference, CatalogRelease, NativeInjectorPolicy};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};
use zip::write::SimpleFileOptions;

pub(super) struct Fixture {
    root: TempDir,
    pub(super) store: Store,
    pub(super) game: PathBuf,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let root = tempdir().expect("fixture");
        let game = root.path().join("game");
        fs::create_dir_all(&game).expect("game folder");
        let game = game.canonicalize().expect("canonical game");
        let store = Store::open(&root.path().join("state/state.sqlite")).expect("state");
        Self { root, store, game }
    }

    pub(super) fn other_game(&self, name: &str) -> PathBuf {
        let game = self.root.path().join(name);
        fs::create_dir_all(&game).expect("other game");
        game.canonicalize().expect("canonical other game")
    }

    pub(super) fn archive(&self, name: &str, file: &str, contents: &[u8]) -> PathBuf {
        let path = self.root.path().join(format!("{name}.zip"));
        let mut zip = zip::ZipWriter::new(File::create(&path).expect("archive"));
        zip.start_file(format!("pack/{file}"), SimpleFileOptions::default())
            .expect("entry");
        zip.write_all(contents).expect("entry bytes");
        zip.finish().expect("archive finish");
        path
    }

    pub(super) fn plan(&self, archive: &Path) -> InstallPlan {
        let checked = inspect(archive).expect("inspect");
        InstallPlan {
            archive_blake3: checked.blake3,
            game_installation: self.game.clone(),
            status: PlanStatus::Reviewable,
            metadata_only: true,
            operations: checked
                .files
                .into_iter()
                .map(|entry| PlannedFile {
                    root: "game".into(),
                    destination: format!(
                        "addons/{}",
                        entry.entry.rsplit('/').next().expect("file name")
                    ),
                    source: entry.entry,
                    bytes: entry.bytes,
                    kind: "file-copy".into(),
                    owned_directory: None,
                })
                .collect(),
            blockers: Vec::new(),
            skipped_generated_files: Vec::new(),
            content_base: None,
            content_root: None,
        overrides: Vec::new(),
}
    }

    pub(super) fn install(
        &mut self,
        name: &str,
        file: &str,
    ) -> crate::transactions::ManagedPackage {
        let archive = self.archive(name, file, name.as_bytes());
        let plan = self.plan(&archive);
        self.store
            .install_operations_zip(&self.game, &archive, &plan)
            .expect("install")
    }

    pub(super) fn install_catalog(
        &mut self,
        name: &str,
        mod_id: &str,
    ) -> (crate::transactions::ManagedPackage, CatalogRelease) {
        let archive = self.archive(name, &format!("{name}.plugin"), name.as_bytes());
        let plan = self.plan(&archive);
        let release = CatalogRelease::new(reference(mod_id), Vec::new());
        let package = self
            .store
            .install_catalog_release(
                &self.game,
                &archive,
                &plan,
                &release,
                NativeInjectorPolicy::Refuse,
            )
            .expect("catalog install");
        (package, release)
    }

    pub(super) fn profiles(&self) -> Vec<Profile> {
        list(&self.store, &self.game).expect("profiles")
    }

    pub(super) fn profile(&self, name: &str) -> Profile {
        self.profiles()
            .into_iter()
            .find(|profile| profile.name == name)
            .expect("profile")
    }

    pub(super) fn counts(&self, table: &str) -> i64 {
        self.store
            .connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("count")
    }
}

fn reference(mod_id: &str) -> CatalogReference {
    CatalogReference {
        bucket: "deps".into(),
        game_id: "test-game".into(),
        mod_id: mod_id.into(),
    }
}
