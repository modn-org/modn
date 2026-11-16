//! Git catalog snapshot reuse for an unchanged commit.

use super::tests::{registration, remote, source, test_key};
use crate::buckets::BucketStore;
use std::fs;
use std::path::{Path, PathBuf};

fn registered_path(state: &Path) -> PathBuf {
    let bytes = fs::read(registration(state)).expect("registration");
    let pointer: serde_json::Value = serde_json::from_slice(&bytes).expect("registration JSON");
    PathBuf::from(pointer["path"].as_str().expect("registered path"))
}

fn snapshots(state: &Path) -> usize {
    fs::read_dir(state.join("buckets"))
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with("git-catalog-")
                })
                .count()
        })
        .unwrap_or_default()
}

#[test]
fn an_unchanged_commit_reuses_the_published_tree() {
    let remote = remote("1", test_key());
    let state = tempfile::tempdir().expect("state");
    let store = BucketStore::new(state.path());
    store
        .sync_git_with_signer("community", Some(source(&remote)))
        .expect("first sync");
    let before = registered_path(state.path());
    assert_eq!(snapshots(state.path()), 1);

    store
        .sync_git_with_signer("community", None)
        .expect("a repeated commit is accepted");

    assert_eq!(registered_path(state.path()), before);
    assert_eq!(snapshots(state.path()), 1, "one active snapshot");
    assert!(before.join("mods/test-game/mod.json").is_file());
}
