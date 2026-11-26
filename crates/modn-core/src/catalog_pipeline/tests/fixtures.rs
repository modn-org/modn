//! A temporary manager profile, bucket and archive fixture.
//!
//! Every test builds its own data and profile directories, so no test reads or
//! writes the user's real Modn state. Roots are explicit, so no fixture reads
//! app data from the environment.

use crate::acquisition::ArchiveCache;
use crate::approval::Capabilities;
use crate::archive;
use crate::buckets::{
    ArchiveFormat, BucketPackage, BucketSnapshot, BucketStore, PackageManifest, PackageSource,
};
use crate::catalog_pipeline::{
    plan_install, CatalogPlan, ConsentInputs, InstallInput, InstallTarget,
};
use crate::manager_roots::ManagerRoots;
use crate::planning::{InstallPlan, PlanStatus, PlannedFile};
use crate::state::Store;
use crate::transactions::{CatalogReference, CatalogRelease, ManagedPackage, NativeInjectorPolicy};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use zip::write::SimpleFileOptions;

pub(super) const GAME: &str = "test-game";

pub(super) struct Fixture {
    data: TempDir,
    profile: TempDir,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let data = tempfile::tempdir().expect("data directory");
        let profile = tempfile::tempdir().expect("profile directory");
        let main = data.path().join("buckets/main");
        fs::create_dir_all(&main).expect("main bucket");
        fs::write(
            main.join("bucket.json"),
            br#"{"schema_version":2,"name":"main","title":"Test main"}"#,
        )
        .expect("bucket metadata");
        Self { data, profile }
    }

    pub(super) fn data(&self) -> &Path {
        self.data.path()
    }

    pub(super) fn roots(&self) -> ManagerRoots {
        ManagerRoots::new(self.data.path(), Some(self.profile.path()))
    }

    pub(super) fn add(&self, manifest: &PackageManifest) {
        let directory = self
            .data
            .path()
            .join("buckets/main/mods")
            .join(&manifest.game);
        fs::create_dir_all(&directory).expect("mods directory");
        fs::write(
            directory.join(format!("{}.json", manifest.id)),
            serde_json::to_vec(manifest).expect("manifest JSON"),
        )
        .expect("manifest");
    }

    pub(super) fn snapshot(&self) -> BucketSnapshot {
        BucketStore::new(&self.roots())
            .snapshot(Some(GAME))
            .expect("snapshot")
    }

    pub(super) fn cache(&self) -> ArchiveCache {
        self.roots().archive_cache()
    }

    pub(super) fn zip(&self, name: &str) -> PathBuf {
        let path = self.data.path().join(format!("{name}.zip"));
        let mut writer = zip::ZipWriter::new(File::create(&path).expect("archive"));
        writer
            .start_file(format!("pack/{name}.txt"), SimpleFileOptions::default())
            .expect("archive entry");
        writer.write_all(name.as_bytes()).expect("entry bytes");
        writer.finish().expect("archive");
        path
    }

    pub(super) fn state_path(&self) -> PathBuf {
        self.roots().state_database()
    }

    pub(super) fn game(&self) -> PathBuf {
        let game = self.data.path().join("game");
        fs::create_dir_all(game.join("addons")).expect("game directory");
        game.canonicalize().expect("canonical game")
    }

    pub(super) fn install(
        &self,
        store: &mut Store,
        game: &Path,
        archive_path: &Path,
        release: &CatalogRelease,
    ) -> ManagedPackage {
        let proposal = install_plan(game, archive_path);
        store
            .install_catalog_release(
                game,
                archive_path,
                &proposal,
                release,
                NativeInjectorPolicy::Refuse,
            )
            .expect("catalog install")
    }
}

/// A minimal reviewable plan that copies each archive entry into `addons/`.
pub(super) fn install_plan(game: &Path, archive_path: &Path) -> InstallPlan {
    let checked = archive::inspect(archive_path).expect("archive inspection");
    InstallPlan {
        archive_blake3: checked.blake3,
        game_installation: game.to_path_buf(),
        status: PlanStatus::Reviewable,
        metadata_only: true,
        operations: checked
            .files
            .into_iter()
            .map(|entry| PlannedFile {
                root: "game".into(),
                destination: format!(
                    "addons/{}",
                    entry.entry.rsplit('/').next().expect("entry name")
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
    }
}

pub(super) fn release(id: &str) -> CatalogRelease {
    CatalogRelease::new(
        CatalogReference::new("main".to_owned(), GAME.to_owned(), id.to_owned()),
        Vec::new(),
    )
}

pub(super) fn reference_of(id: &str) -> String {
    format!("main/{GAME}/{id}")
}

pub(super) fn readonly_store(fixture: &Fixture) -> Store {
    Store::open_readonly(&fixture.state_path()).expect("read-only state")
}

pub(super) fn plan_for(
    fixture: &Fixture,
    snapshot: &BucketSnapshot,
    game: &Path,
    id: &str,
    store: Option<&Store>,
    supplied: &[PathBuf],
    flags: Capabilities,
) -> CatalogPlan {
    let selected = package(snapshot, id);
    let cache = fixture.cache();
    let input = InstallInput::new(
        &selected,
        snapshot,
        store,
        InstallTarget::new(GAME, game),
        supplied,
        &cache,
        crate::acquisition::CacheUse::Reuse,
        ConsentInputs::new(flags),
    );
    plan_install(&input).expect("plan")
}

/// A stable short label for one refusal, in the order the install checks them.
pub(super) fn refusal_label(refusal: &crate::catalog_pipeline::PlanRefusal) -> String {
    use crate::catalog_pipeline::PlanRefusal;
    match refusal {
        PlanRefusal::NativeInjector { reference } => format!("native {reference}"),
        PlanRefusal::ApprovalRequired { refusal } => format!(
            "approval {}",
            refusal.as_json()["package"].as_str().unwrap_or_default()
        ),
        PlanRefusal::Conflict { conflict } => format!("conflict {}", conflict.reference),
        PlanRefusal::DependencyNotReady { reference, .. } => format!("dependency {reference}"),
        PlanRefusal::ArchiveRequired { reference, .. } => format!("archive {reference}"),
        PlanRefusal::ArchiveUnmatched { path } => format!("unmatched {}", path.display()),
        _ => "other".to_owned(),
    }
}

/// Every file under `root` with its bytes, keyed by its path relative to `root`.
pub(super) fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, directory: &Path, found: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(directory).expect("directory") {
            let entry = entry.expect("entry");
            let path = entry.path();
            let metadata = entry.metadata().expect("metadata");
            if metadata.is_dir() {
                walk(root, &path, found);
            } else if metadata.is_file() {
                found.insert(
                    path.strip_prefix(root).expect("relative").to_path_buf(),
                    fs::read(&path).expect("bytes"),
                );
            }
        }
    }
    let mut found = BTreeMap::new();
    walk(root, root, &mut found);
    found
}

pub(super) fn package(snapshot: &BucketSnapshot, id: &str) -> BucketPackage {
    snapshot
        .packages()
        .iter()
        .find(|package| package.package.id == id)
        .cloned()
        .expect("published release")
}

pub(super) fn manifest(id: &str, pin: Option<&str>) -> PackageManifest {
    PackageManifest {
        schema_version: 2,
        id: id.to_owned(),
        game: GAME.to_owned(),
        name: id.to_owned(),
        version: "1.0".to_owned(),
        archive_format: pin.map(|_| ArchiveFormat::Zip),
        blake3: pin.map(str::to_owned),
        sources: vec![PackageSource::Https {
            url: format!("https://example.test/{id}.zip"),
        }],
        redistributable: false,
        upstream_sha256: None,
        dependencies: Vec::new(),
        homepage: None,
        executables: Vec::new(),
        dependency_versions: BTreeMap::new(),
        conflicts: Vec::new(),
        native_injector: false,
    }
}

pub(super) fn git_manifest(id: &str, commit: &str) -> PackageManifest {
    let mut manifest = manifest(id, None);
    manifest.sources = vec![PackageSource::Git {
        repository: "owner/project".to_owned(),
        commit: commit.to_owned(),
    }];
    manifest
}

pub(super) fn digest(path: &Path) -> String {
    crate::archive::inspect(path)
        .expect("archive inspection")
        .blake3
}
