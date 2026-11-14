#![allow(
    dead_code,
    reason = "each integration-test crate uses a different subset of these helpers"
)]
use crate::git_fixture::file_url;
use crate::main_bucket;
use crate::support::{error, Fixture};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

pub fn write_bucket(root: &Path, commit: &str) {
    let bucket = root.join("git-fixture");
    let game = bucket.join("mods/test-game");
    fs::create_dir_all(&game).expect("bucket directory");
    fs::write(
        bucket.join("bucket.json"),
        br#"{"schema_version":2,"name":"git-fixture","title":"Git fixture"}"#,
    )
    .expect("bucket metadata");
    let manifest = json!({
        "schema_version": 2,
        "id": "git-package",
        "game": "test-game",
        "name": "Git Package",
        "version": "1.0",
        "sources": [{"kind": "git", "repository": "fixture/project", "commit": commit}],
    });
    fs::write(
        game.join("git-package.json"),
        serde_json::to_vec(&manifest).expect("manifest JSON"),
    )
    .expect("manifest");
}

pub fn add_bucket(fixture: &Fixture, root: &Path, commit: &str) {
    write_bucket(root, commit);
    let bucket = root.join("git-fixture");
    let result = fixture.run_paths(&["bucket", "add", "git-fixture"], &[&bucket]);
    assert!(result.status.success(), "{}", error(&result));
}

pub fn run(fixture: &Fixture, base: &Path, args: &[&str]) -> Output {
    main_bucket::command(env!("CARGO_BIN_EXE_modn"))
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .env("MODN_TEST_GIT_REMOTE_BASE", file_url(base))
        .args(args)
        .output()
        .expect("run CLI")
}

pub fn installed_plugin(fixture: &Fixture) -> PathBuf {
    fixture.game.join("addons/git-source.plugin")
}

pub fn cache_archives(fixture: &Fixture) -> Vec<PathBuf> {
    let cache = fixture.state.join("cache");
    match fs::read_dir(cache) {
        Ok(entries) => entries
            .map(|entry| entry.expect("cache entry").path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "zip"))
            .collect(),
        Err(_) => Vec::new(),
    }
}
