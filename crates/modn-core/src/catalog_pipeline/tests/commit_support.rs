//! A temporary manager profile, catalog and fake acquisition for commit tests.
//!
//! Every test builds its own data and profile directories, so no test reads or
//! writes the user's real Modn state. The app-data variables point at one
//! temporary directory for the whole test binary.

use super::fixtures;
use crate::acquisition::{
    CacheUse, GitAcquired, GitCacheKey, NexusDownloadUrl, PlannedAction, ReleaseResolver,
    SourceBody, SourceCandidate, SourcePolicy, SourceTransport,
};
use crate::buckets::{BucketPackage, BucketSnapshot, BucketStore, PackageManifest, PackageSource};
use crate::catalog_pipeline::{
    plan_digest, preview_catalog_install, CatalogPlan, Consent, ConsentInputs, InstallInput,
    InstallTarget,
};
use crate::manager_roots::ManagerRoots;
use crate::progress::OperationControl;
use crate::state::Store;
use anyhow::Result;
use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, File};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tempfile::TempDir;
use zip::write::SimpleFileOptions;

pub(super) const GAME: &str = "test-game";

pub(super) struct CommitFixture {
    data: TempDir,
    _profile: TempDir,
    module: TempDir,
    roots: ManagerRoots,
    game: PathBuf,
}

impl CommitFixture {
    pub(super) fn new() -> Self {
        isolate_environment();
        let data = tempfile::tempdir().expect("data directory");
        let profile = tempfile::tempdir().expect("profile directory");
        let module = tempfile::tempdir().expect("module directory");
        let main = profile.path().join("buckets/main");
        fs::create_dir_all(&main).expect("main bucket");
        fs::write(
            main.join("bucket.json"),
            br#"{"schema_version":2,"name":"main","title":"Test main"}"#,
        )
        .expect("bucket metadata");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/test-game");
        copy_tree(&source, module.path());
        let roots = ManagerRoots::new(data.path(), Some(profile.path()))
            .with_development_modules(Some(module.path().to_path_buf()));
        let game = data.path().join("game");
        fs::create_dir_all(game.join("bin")).expect("game directory");
        fs::write(game.join("bin/TestGame.exe"), b"game-exe").expect("marker");
        let game = game.canonicalize().expect("canonical game");
        Self {
            data,
            _profile: profile,
            module,
            roots,
            game,
        }
    }

    pub(super) fn roots(&self) -> &ManagerRoots {
        &self.roots
    }

    pub(super) fn game(&self) -> &Path {
        &self.game
    }

    pub(super) fn module(&self) -> &Path {
        self.module.path()
    }

    pub(super) fn cache(&self) -> crate::acquisition::ArchiveCache {
        self.roots.archive_cache()
    }

    pub(super) fn add(&self, manifest: &PackageManifest) {
        let directory = self
            .roots
            .profile()
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
        BucketStore::new(&self.roots)
            .snapshot(Some(GAME))
            .expect("snapshot")
    }

    pub(super) fn plugin_zip(&self, name: &str) -> PathBuf {
        self.zip(name, &format!("package/plugins/{name}.plugin"))
    }

    /// A plugin archive at `file` with fixed `payload`, so two versions of one
    /// mod differ in bytes while they share the entry name.
    pub(super) fn versioned_plugin(&self, file: &str, id: &str, payload: &str) -> PathBuf {
        let path = self.data.path().join(file);
        let mut writer = zip::ZipWriter::new(File::create(&path).expect("archive"));
        writer
            .start_file(
                format!("package/plugins/{id}.plugin"),
                SimpleFileOptions::default(),
            )
            .expect("archive entry");
        writer.write_all(payload.as_bytes()).expect("entry bytes");
        writer.finish().expect("archive");
        path
    }

    pub(super) fn raw_zip(&self, name: &str) -> PathBuf {
        self.zip(name, &format!("pack/{name}.txt"))
    }

    fn zip(&self, name: &str, entry: &str) -> PathBuf {
        let path = self.data.path().join(format!("{name}.zip"));
        let mut writer = zip::ZipWriter::new(File::create(&path).expect("archive"));
        writer
            .start_file(entry, SimpleFileOptions::default())
            .expect("archive entry");
        writer.write_all(name.as_bytes()).expect("entry bytes");
        writer.finish().expect("archive");
        path
    }

    pub(super) fn state_path(&self) -> PathBuf {
        self.roots.state_database()
    }

    pub(super) fn open_store(&self) -> Store {
        Store::open(&self.state_path()).expect("writable state")
    }

    pub(super) fn readonly_store(&self) -> Store {
        Store::open_readonly(&self.state_path()).expect("read-only state")
    }

    pub(super) fn plan(
        &self,
        snapshot: &BucketSnapshot,
        id: &str,
        store: Option<&Store>,
        supplied: &[PathBuf],
    ) -> CatalogPlan {
        let selected = package(snapshot, id);
        let cache = self.cache();
        let input = InstallInput::new(
            &selected,
            snapshot,
            store,
            InstallTarget::new(GAME, &self.game),
            supplied,
            &cache,
            CacheUse::Reuse,
            ConsentInputs::default(),
        );
        crate::catalog_pipeline::plan_install(&input).expect("plan")
    }

    pub(super) fn local_archives(
        &self,
        archives: &[(&str, &PathBuf)],
    ) -> BTreeMap<String, PathBuf> {
        archives
            .iter()
            .map(|(id, path)| (reference_of(id), (*path).clone()))
            .collect()
    }

    pub(super) fn preview(
        &self,
        plan: &CatalogPlan,
        archives: &BTreeMap<String, PathBuf>,
    ) -> Vec<crate::catalog_pipeline::ReleasePreview> {
        preview_catalog_install(&self.roots, plan, archives, &[]).expect("preview")
    }

    pub(super) fn consent(
        &self,
        plan: &CatalogPlan,
        previews: &[crate::catalog_pipeline::ReleasePreview],
    ) -> Consent {
        Consent::new(false, Vec::new(), plan_digest(plan, &[], previews))
    }

    pub(super) fn count(&self, table: &str) -> i64 {
        let connection = rusqlite::Connection::open(self.state_path()).expect("state");
        connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("count")
    }

    pub(super) fn history(&self) -> Vec<crate::history::HistoryOperation> {
        Store::open_readonly(&self.state_path())
            .expect("read-only state")
            .history(Some(GAME))
            .expect("history")
    }
}

/// Point every manager app-data variable at one temporary directory.
fn isolate_environment() {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    let root = ROOT.get_or_init(|| tempfile::tempdir().expect("appdata directory").keep());
    for name in ["LOCALAPPDATA", "APPDATA", "MODN_APPDATA"] {
        std::env::set_var(name, root);
    }
}

fn copy_tree(source: &Path, target: &Path) {
    for entry in fs::read_dir(source).expect("module source") {
        let entry = entry.expect("module entry");
        let path = entry.path();
        let destination = target.join(entry.file_name());
        if entry.metadata().expect("module metadata").is_dir() {
            fs::create_dir_all(&destination).expect("module directory");
            copy_tree(&path, &destination);
        } else {
            fs::copy(&path, &destination).expect("module file");
        }
    }
}

/// A transport that answers one queued body per URL and records every call.
#[derive(Default)]
pub(super) struct FakeTransport {
    replies: RefCell<BTreeMap<String, VecDeque<Vec<u8>>>>,
    calls: RefCell<Vec<String>>,
}

impl FakeTransport {
    pub(super) fn respond(&self, url: &str, bytes: &[u8]) {
        self.replies
            .borrow_mut()
            .entry(url.to_owned())
            .or_default()
            .push_back(bytes.to_vec());
    }

    pub(super) fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
}

impl SourceTransport for FakeTransport {
    fn open(
        &self,
        url: &str,
        _policy: &SourcePolicy,
    ) -> std::result::Result<SourceBody, crate::acquisition::FetchError> {
        self.calls.borrow_mut().push(url.to_owned());
        let mut replies = self.replies.borrow_mut();
        let queue = replies
            .get_mut(url)
            .expect("the acquisition opened an unqueued URL");
        let body = queue.pop_front().expect("a queued transport reply");
        Ok(SourceBody::new(Box::new(Cursor::new(body)), None))
    }
}

/// A resolver that supports only plain fetch sources.
pub(super) struct FetchResolver;

impl ReleaseResolver for FetchResolver {
    fn git(
        &self,
        _release: &BucketPackage,
        _cache: &crate::acquisition::ArchiveCache,
        _source: &PackageSource,
        _key: &GitCacheKey<'_>,
        _cache_use: CacheUse,
        _control: &OperationControl,
    ) -> Result<GitAcquired> {
        anyhow::bail!("the fake resolver does not fetch Git sources")
    }

    fn nexus(&self, _release: &BucketPackage, _source: &PackageSource) -> Result<NexusDownloadUrl> {
        anyhow::bail!("the fake resolver does not fetch Nexus sources")
    }

    fn open_nexus(&self, _link: &NexusDownloadUrl) -> Result<SourceBody> {
        anyhow::bail!("the fake resolver does not open Nexus links")
    }

    fn candidate(&self, _release: &BucketPackage, action: PlannedAction) -> SourceCandidate {
        match action {
            PlannedAction::Fetch { candidate } => candidate,
            PlannedAction::Github { expected, .. } => SourceCandidate::fetch(expected),
            _ => SourceCandidate::refused("unknown source", "unknown release source"),
        }
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
    fixtures::manifest(id, pin)
}

pub(super) fn digest(path: &Path) -> String {
    crate::archive::inspect(path)
        .expect("archive inspection")
        .blake3
}

pub(super) fn reference_of(id: &str) -> String {
    format!("main/{GAME}/{id}")
}
