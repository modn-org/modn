#![allow(
    dead_code,
    reason = "shared fixture exposes helpers for different test crates"
)]

#[path = "git_command.rs"]
mod git_command;

use minisign::KeyPair;
use modn_core::buckets::{catalog_statement, GitCatalogSource};
use serde_json::json;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

pub fn git(root: &Path, arguments: &[&str]) -> String {
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

pub fn commit(root: &Path, message: &str) {
    git(root, &["add", "."]);
    git(root, &["commit", "-m", message]);
}

pub fn package() -> String {
    json!({"schema_version":2,"id":"mod","game":"test-game","name":"Mod","version":"1",
        "archive_format":"zip","blake3":"0".repeat(64),
        "sources":[{"kind":"manual","filename":"mod.zip"}]})
    .to_string()
}

pub fn copy_tree(source: &Path, target: &Path) {
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

pub fn write_signature(root: &Path, pair: &KeyPair, public_text: &str) {
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

pub fn registration(state: &Path) -> PathBuf {
    state.join("buckets/registrations/community.json")
}

pub struct SignedRemote {
    directory: tempfile::TempDir,
    pair: KeyPair,
    public: String,
}

impl SignedRemote {
    pub fn path(&self) -> &Path {
        self.directory.path()
    }

    pub fn resign(&self, message: &str) {
        write_signature(self.path(), &self.pair, &self.public);
        commit(self.path(), message);
    }

    pub fn source(&self) -> GitCatalogSource {
        GitCatalogSource::new(
            self.path().to_string_lossy().into_owned(),
            "fixture".to_owned(),
        )
    }
}

pub fn signed_remote(layout: impl FnOnce(&Path)) -> SignedRemote {
    let directory = tempfile::tempdir().expect("remote");
    git(directory.path(), &["init", "--initial-branch=fixture"]);
    fs::write(
        directory.path().join("bucket.json"),
        r#"{"schema_version":2,"name":"community","title":"Community"}"#,
    )
    .expect("metadata");
    fs::create_dir_all(directory.path().join("mods/test-game")).expect("mods");
    fs::write(directory.path().join("mods/test-game/mod.json"), package()).expect("package");
    layout(directory.path());
    let pair = KeyPair::generate_unencrypted_keypair().expect("key");
    let public = pair.pk.to_box().expect("public key").into_string();
    write_signature(directory.path(), &pair, &public);
    commit(directory.path(), "fixture");
    SignedRemote {
        directory,
        pair,
        public,
    }
}

pub fn write_module_files(root: &Path, game: &str, api: u32) {
    let directory = root.join("games").join(game);
    fs::create_dir_all(&directory).expect("game module directory");
    fs::write(
        directory.join("module.json"),
        serde_json::json!({
            "game_id": game,
            "version": "1",
            "api_version": api,
            "entrypoint": "module.lua",
        })
        .to_string(),
    )
    .expect("module manifest");
    fs::write(
        directory.join("module.lua"),
        format!("return {{game_id='{game}', api_version={api}}}"),
    )
    .expect("module script");
}
