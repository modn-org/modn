#![allow(
    dead_code,
    reason = "shared fixture exposes helpers for different test crates"
)]
use minisign::KeyPair;
use modn_core::buckets::{catalog_statement, CatalogPublicKey};
use serde_json::json;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A signed Git catalog fixture that generates its signing key in process.
pub struct ModuleCatalog {
    directory: tempfile::TempDir,
    pair: KeyPair,
    public: String,
    key_id: String,
}

impl ModuleCatalog {
    pub fn new(name: &str) -> Self {
        let directory = tempfile::tempdir().expect("catalog directory");
        git(directory.path(), &["init", "--initial-branch=main"]);
        fs::write(
            directory.path().join("bucket.json"),
            json!({"schema_version": 2, "name": name, "title": "Community fixture"}).to_string(),
        )
        .expect("bucket metadata");
        let pair = KeyPair::generate_unencrypted_keypair().expect("fixture signing key");
        let public = pair.pk.to_box().expect("public key").into_string();
        let key_id = CatalogPublicKey::parse(&public)
            .expect("catalog public key")
            .key_id()
            .to_string();
        let catalog = Self {
            directory,
            pair,
            public,
            key_id,
        };
        catalog.sign_and_commit("catalog");
        catalog
    }

    pub fn repo(&self) -> &Path {
        self.directory.path()
    }

    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    pub fn write_module(&self, game: &str, version: &str, lua: &str) -> PathBuf {
        let directory = self.directory.path().join("games").join(game);
        fs::create_dir_all(&directory).expect("game module directory");
        fs::write(
            directory.join("module.json"),
            json!({
                "game_id": game,
                "version": version,
                "api_version": 1,
                "entrypoint": "module.lua",
            })
            .to_string(),
        )
        .expect("module manifest");
        fs::write(directory.join("module.lua"), lua).expect("module Lua");
        directory
    }

    pub fn sign_and_commit(&self, message: &str) -> String {
        let staging = tempfile::tempdir().expect("signed catalog staging");
        copy_tree(self.directory.path(), staging.path());
        let statement = catalog_statement(staging.path()).expect("catalog statement");
        let signature = minisign::sign(
            Some(&self.pair.pk),
            &self.pair.sk,
            Cursor::new(statement.as_bytes()),
            Some("catalog test"),
            Some("catalog test signature"),
        )
        .expect("catalog signature")
        .into_string();
        fs::write(self.directory.path().join("catalog.pub"), &self.public).expect("public key");
        fs::write(self.directory.path().join("catalog.sig"), signature).expect("signature");
        commit(self.directory.path(), message)
    }
}

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).expect("copy directory");
    for entry in fs::read_dir(source).expect("copy entries") {
        let entry = entry.expect("copy entry");
        if entry.file_name() == ".git" {
            continue;
        }
        let target = target.join(entry.file_name());
        if entry.file_type().expect("copy type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy file");
        }
    }
}

fn git(root: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
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

fn commit(root: &Path, message: &str) -> String {
    git(root, &["add", "."]);
    git(root, &["commit", "-m", message]);
    git(root, &["rev-parse", "HEAD"])
}

/// Minimal Lua policy that returns a valid module identity.
pub fn identity_lua(game: &str) -> String {
    format!("return {{game_id='{game}', api_version=1}}")
}

/// Minimal Lua policy that validates a marker file and reports an empty scan.
pub fn policy_lua(game: &str, marker: &str) -> String {
    format!(
        r#"local policy = {{
    game_id = '{game}',
    api_version = mods_host.api_version,
    marker = '{marker}',
}}
function policy.validate()
    local ok, item = pcall(mods_host.fs.metadata, 1, policy.marker)
    return ok and item ~= nil and item.kind == 'file'
end
function policy.scan(limit)
    return {{ mods = {{}}, framework_signals = {{}}, incomplete = false,
        scan_issues = {{}}, redmod_available = false }}
end
function policy.doctor(report)
    return {{}}
end
return policy
"#
    )
}
