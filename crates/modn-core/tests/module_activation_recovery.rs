#[path = "support/git_command.rs"]
mod git_command;
#[path = "support/main_bucket.rs"]
mod main_bucket;

use minisign::KeyPair;
use modn_core::buckets::{
    catalog_statement, BucketStore, CatalogPublicKey, GitCatalogSource, GitCatalogSync,
    ModuleTrustAck,
};
use modn_core::game_modules::{ModuleRegistry, RegisteredModule};
use modn_core::manager_roots::ManagerRoots;
use serde_json::json;
use std::ffi::OsString;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

const BUCKET: &str = "community";
const GAME: &str = "alpha-game";
const OLD_VERSION: &str = "1.0.0";
const NEW_VERSION: &str = "2.0.0";
const API_VERSION: u32 = 3;
const ISOLATED_VARIABLES: [&str; 3] = ["LOCALAPPDATA", "APPDATA", "MODN_APPDATA"];

/// Point every manager app-data variable at a temporary directory.
///
/// The test builds `ManagerRoots` explicitly, but no entry point may fall back
/// to the user's real Modn state.
struct IsolatedEnvironment {
    previous: Vec<(&'static str, Option<OsString>)>,
}

impl IsolatedEnvironment {
    fn new(directory: &Path) -> Self {
        let mut previous = Vec::with_capacity(ISOLATED_VARIABLES.len());
        for name in ISOLATED_VARIABLES {
            previous.push((name, std::env::var_os(name)));
            std::env::set_var(name, directory);
        }
        Self { previous }
    }
}

impl Drop for IsolatedEnvironment {
    fn drop(&mut self) {
        for (name, value) in self.previous.drain(..).rev() {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}

fn git(root: &Path, arguments: &[&str]) -> String {
    let output = git_command::git_command()
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
        ])
        .args(arguments)
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn commit(root: &Path, message: &str) {
    git(root, &["add", "."]);
    git(root, &["commit", "-m", message]);
}

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).expect("copy directory");
    for entry in fs::read_dir(source).expect("copy entries") {
        let entry = entry.expect("entry");
        if entry.file_name() == ".git" {
            continue;
        }
        let target = target.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy file");
        }
    }
}

fn write_signature(root: &Path, pair: &KeyPair, public_text: &str) {
    let staging = tempfile::tempdir().expect("staging");
    copy_tree(root, staging.path());
    let statement = catalog_statement(staging.path()).expect("catalog statement");
    let signature = minisign::sign(
        Some(&pair.pk),
        &pair.sk,
        Cursor::new(statement.as_bytes()),
        Some("catalog test"),
        Some("catalog test signature"),
    )
    .expect("sign")
    .into_string();
    fs::write(root.join("catalog.pub"), public_text).expect("public key");
    fs::write(root.join("catalog.sig"), signature).expect("signature");
}

struct Catalog {
    directory: tempfile::TempDir,
    pair: KeyPair,
    public: String,
    key_id: String,
}

impl Catalog {
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("remote");
        git(directory.path(), &["init", "--initial-branch=fixture"]);
        fs::write(
            directory.path().join("bucket.json"),
            json!({"schema_version": 2, "name": BUCKET, "title": "Community"}).to_string(),
        )
        .expect("metadata");
        let pair = KeyPair::generate_unencrypted_keypair().expect("key");
        let public = pair.pk.to_box().expect("public key").into_string();
        let key_id = CatalogPublicKey::parse(&public)
            .expect("catalog key")
            .key_id()
            .to_string();
        let catalog = Self {
            directory,
            pair,
            public,
            key_id,
        };
        catalog.sign_and_commit("first");
        catalog
    }

    fn module_directory(&self) -> PathBuf {
        self.directory.path().join("games").join(GAME)
    }

    fn write_module(&self, version: &str) {
        let directory = self.module_directory();
        fs::create_dir_all(&directory).expect("game module directory");
        fs::write(
            directory.join("module.json"),
            json!({
                "game_id": GAME,
                "version": version,
                "api_version": API_VERSION,
                "entrypoint": "module.lua",
            })
            .to_string(),
        )
        .expect("manifest");
        fs::write(
            directory.join("module.lua"),
            format!("return {{game_id='{GAME}', api_version={API_VERSION}, version='{version}'}}"),
        )
        .expect("Lua");
    }

    fn sign_and_commit(&self, message: &str) {
        write_signature(self.directory.path(), &self.pair, &self.public);
        commit(self.directory.path(), message);
    }

    fn source(&self) -> GitCatalogSource {
        GitCatalogSource::new(
            self.directory.path().to_string_lossy().into_owned(),
            "fixture".to_owned(),
        )
    }

    fn sync(&self, store: &BucketStore) -> GitCatalogSync {
        store
            .sync_git_with_signer(BUCKET, Some(self.source()))
            .expect("sync")
    }

    fn trust(&self, store: &BucketStore) {
        store
            .trust_modules(
                BUCKET,
                ModuleTrustAck::Acknowledged {
                    expected_fingerprint: Some(&self.key_id),
                },
            )
            .expect("trust");
    }
}

fn digest(manifest: &[u8], script: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(manifest);
    hasher.update(script);
    hasher.finalize().to_hex().to_string()
}

fn versions(releases: &[RegisteredModule]) -> Vec<(String, bool)> {
    releases
        .iter()
        .map(|release| (release.module.version.clone(), release.active))
        .collect()
}

fn retained_file(roots: &ManagerRoots, name: &str) -> Vec<u8> {
    fs::read(
        roots
            .module_registry()
            .join(GAME)
            .join("versions")
            .join(NEW_VERSION)
            .join(name),
    )
    .expect("retained release file")
}

/// Build the registry state that a process death leaves between the two loops
/// of `apply`: every version directory is moved, but the `current` pointer is
/// still the old release. Returns the published manifest and script bytes.
fn leave_release_moved_but_inactive(roots: &ManagerRoots, catalog: &Catalog) -> (Vec<u8>, Vec<u8>) {
    let published = catalog.module_directory();
    let manifest = fs::read(published.join("module.json")).expect("published manifest");
    let script = fs::read(published.join("module.lua")).expect("published script");
    let version_directory = roots
        .module_registry()
        .join(GAME)
        .join("versions")
        .join(NEW_VERSION);
    fs::create_dir_all(&version_directory).expect("version directory");
    fs::write(version_directory.join("module.json"), &manifest).expect("moved manifest");
    fs::write(version_directory.join("module.lua"), &script).expect("moved script");
    let receipt = json!({
        "game_id": GAME,
        "version": NEW_VERSION,
        "api_version": API_VERSION,
        "blake3": digest(&manifest, &script),
        "bucket": BUCKET,
        "signer_key_id": catalog.key_id,
    });
    fs::write(
        version_directory.join("registration.json"),
        serde_json::to_vec(&receipt).expect("receipt"),
    )
    .expect("moved receipt");
    (manifest, script)
}

#[test]
#[ignore = "reproduces issue #167: a death between the version moves and the current write leaves the newer release inactive; the activation fix removes the ignore"]
fn process_death_between_moves_and_current_write_activates_the_newer_release() {
    let isolation = tempfile::tempdir().expect("isolation");
    let _environment = IsolatedEnvironment::new(isolation.path());
    main_bucket::seeded_roots(|roots| {
        let data = roots.data();
        let store = BucketStore::new(&ManagerRoots::new(data, None));
        let catalog = Catalog::new();
        catalog.sync(&store);
        catalog.trust(&store);
        catalog.write_module(OLD_VERSION);
        catalog.sign_and_commit("v1");
        let synced = catalog.sync(&store);
        assert_eq!(synced.modules.imported.len(), 1);
        let registry = ModuleRegistry::new(roots);
        let active = registry.active(GAME).expect("active release");
        assert_eq!(
            active.map(|release| release.module.version).as_deref(),
            Some(OLD_VERSION)
        );

        catalog.write_module(NEW_VERSION);
        catalog.sign_and_commit("v2");
        let (manifest, script) = leave_release_moved_but_inactive(roots, &catalog);

        let pointer = fs::read_to_string(roots.module_registry().join(GAME).join("current"))
            .expect("current pointer");
        assert_eq!(
            pointer.trim_end_matches(['\r', '\n']),
            OLD_VERSION,
            "the simulated death must leave the current pointer on the old release"
        );
        assert_eq!(
            retained_file(roots, "module.json"),
            manifest,
            "the simulated death must leave the published manifest bytes in the new version directory"
        );
        assert_eq!(
            retained_file(roots, "module.lua"),
            script,
            "the simulated death must leave the published script bytes in the new version directory"
        );
        let retained = registry.releases(GAME).expect("retained releases");
        assert_eq!(
            versions(&retained),
            vec![
                (OLD_VERSION.to_owned(), true),
                (NEW_VERSION.to_owned(), false)
            ],
            "the hand-built release must read back as a valid retained release"
        );
        let newer = retained
            .iter()
            .find(|release| release.module.version == NEW_VERSION)
            .expect("newer release");
        assert_eq!(
            newer.blake3,
            digest(&manifest, &script),
            "the hand-built release must carry the published digest"
        );

        store
            .sync_git_with_signer(BUCKET, None)
            .expect("second sync");

        let active = registry.active(GAME).expect("active release after sync");
        assert_eq!(
            active.map(|release| release.module.version).as_deref(),
            Some(NEW_VERSION),
            "the next sync must activate the newer retained release that a death left behind the current pointer (issue #167)"
        );
    });
}
