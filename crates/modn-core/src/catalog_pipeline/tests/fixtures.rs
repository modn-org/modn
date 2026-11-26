//! A temporary manager profile, bucket and archive fixture.
//!
//! Every test builds its own data and profile directories, so no test reads or
//! writes the user's real Modn state. Roots are explicit, so no fixture reads
//! app data from the environment.

use crate::acquisition::ArchiveCache;
use crate::buckets::{
    ArchiveFormat, BucketPackage, BucketSnapshot, BucketStore, PackageManifest, PackageSource,
};
use crate::manager_roots::ManagerRoots;
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
