#![allow(
    dead_code,
    reason = "shared fixture exposes helpers for different test crates"
)]
use crate::catalog_authoring::{copy_tree, init_repo, path_text, snapshot, stderr};
use crate::main_bucket;
use crate::zip_catalog_fixture::Fixture;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

pub fn registration(state: &Path) -> Value {
    serde_json::from_slice(
        &fs::read(state.join("buckets/registrations/community.json")).expect("registration"),
    )
    .expect("registration JSON")
}

pub fn published_catalog(state: &Path) -> PathBuf {
    let value = registration(state);
    PathBuf::from(value["path"].as_str().expect("published path"))
}

pub fn state_snapshot(state: &Path) -> (Vec<u8>, Vec<(String, Vec<u8>)>) {
    (
        fs::read(state.join("buckets/registrations/community.json")).expect("registration bytes"),
        snapshot(&published_catalog(state)),
    )
}

pub fn entry<'a>(entries: &'a [Value], name: &str) -> &'a Value {
    entries
        .iter()
        .find(|entry| entry["name"] == name)
        .expect("registered bucket")
}

pub fn listed_entry(fixture: &Fixture) -> Value {
    let output = fixture.success(&["--json", "bucket", "list"]);
    let entries: Vec<Value> = serde_json::from_str(&output).expect("bucket list JSON");
    entry(&entries, "community").clone()
}

pub fn copy_into(source: &Path, target: &Path) {
    for entry in fs::read_dir(target).expect("target entries") {
        let entry = entry.expect("target entry");
        if entry.file_name() == ".git" {
            continue;
        }
        if entry.file_type().expect("target type").is_dir() {
            fs::remove_dir_all(entry.path()).expect("remove old directory");
        } else {
            fs::remove_file(entry.path()).expect("remove old file");
        }
    }
    copy_tree(source, target);
}

pub struct GitFixture {
    pub temp: TempDir,
    pub state: PathBuf,
    pub data: PathBuf,
    pub repo: PathBuf,
}

impl GitFixture {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().expect("fixture");
        let data = temp.path().join("data");
        fs::create_dir_all(&data).expect("data");
        main_bucket::seed(&data);
        let repo = init_repo(temp.path());
        let state = temp.path().join("state");
        main_bucket::seed_state(&state);
        Self {
            temp,
            state,
            data,
            repo,
        }
    }

    pub fn run(&self, args: &[&str]) -> Output {
        Command::new(main_bucket::modn_binary())
            .env("LOCALAPPDATA", &self.data)
            .env(
                "MODN_TEST_MAIN_BUCKET_PUBLIC_KEY",
                main_bucket::test_public_key(),
            )
            .env_remove("MODN_GAME_MODULES_DIR")
            .args(["--state-dir", path_text(&self.state)])
            .args(args)
            .output()
            .expect("run fixture command")
    }

    pub fn sync(&self) -> Output {
        self.run(&[
            "--json",
            "bucket",
            "sync",
            "community",
            "--source",
            path_text(&self.repo),
            "--revision",
            "HEAD",
        ])
    }

    pub fn snapshot(&self) -> (Vec<u8>, Vec<(String, Vec<u8>)>) {
        state_snapshot(&self.state)
    }
}

pub fn listed_entry_git(fixture: &GitFixture) -> Value {
    let output = fixture.run(&["--json", "bucket", "list"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let entries: Vec<Value> = serde_json::from_slice(&output.stdout).expect("bucket list JSON");
    entry(&entries, "community").clone()
}
