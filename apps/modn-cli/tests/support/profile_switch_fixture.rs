//! Helpers for the `modn profile switch` CLI tests.
//!
//! Every install copies its archive into the fixture cache, so a later switch
//! can restore the bytes that the recorded claim names.

use crate::profile_fixture::ProfileFixture;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

pub const ALPHA: &str = "archive/pc/mod/alpha.archive";
pub const BETA: &str = "archive/pc/mod/beta.archive";
pub const USER: &str = "archive/pc/mod/user.cfg";

/// Installs one local archive into the active profile and caches its bytes.
pub fn install(fixture: &ProfileFixture, name: &str) -> String {
    let report = fixture.install(name);
    let root = fixture.state().parent().expect("fixture root");
    let cache = fixture.state().join("cache");
    fs::create_dir_all(&cache).expect("cache directory");
    fs::copy(
        root.join("archives").join(format!("{name}.zip")),
        cache.join(format!("cyberpunk-2077#{name}#1.0#0000000.zip")),
    )
    .expect("cache entry");
    report["id"].as_str().expect("package id").to_owned()
}

/// Creates an inactive empty `second` profile.
pub fn create_second(fixture: &ProfileFixture) {
    fixture.json(&["profile", "create", "second"]);
}

/// Installs `beta` into an inactive `second` profile and returns to `default`.
pub fn second_with_beta(fixture: &ProfileFixture, alpha: &str) -> String {
    create_second(fixture);
    fixture.json(&["profile", "switch", "second", "--yes"]);
    let beta = install(fixture, "beta");
    assert_ne!(beta, alpha);
    fixture.json(&["profile", "switch", "default", "--yes"]);
    beta
}

/// The last error code on a progress-mode stderr stream.
pub fn progress_error_code(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|value| value["error"].as_str().map(str::to_owned))
        .next_back()
        .expect("error code")
}

/// Every file and directory below the game installation.
pub fn snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    let mut entries = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).expect("game directory") {
            let path = entry.expect("entry").path();
            let relative = path
                .strip_prefix(root)
                .expect("relative path")
                .to_path_buf();
            if fs::symlink_metadata(&path).expect("metadata").is_dir() {
                pending.push(path);
                entries.insert(relative, None);
            } else {
                entries.insert(relative, Some(fs::read(&path).expect("file bytes")));
            }
        }
    }
    entries
}

/// The name of the profile that `profile list` marks active.
pub fn active_profile(fixture: &ProfileFixture) -> String {
    let list = fixture.profile_list();
    list["profiles"]
        .as_array()
        .expect("profiles")
        .iter()
        .find(|profile| profile["active"] == true)
        .expect("active profile")["name"]
        .as_str()
        .expect("name")
        .to_owned()
}

pub fn digest(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}
