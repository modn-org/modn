use super::{git, BucketStore, GitCatalogSource};
use std::fs;
use std::path::Path;

fn commit(root: &Path, title: &str) -> String {
    git(root, &["add", "."]).unwrap();
    git(
        root,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-m",
            title,
        ],
    )
    .unwrap();
    git(root, &["rev-parse", "HEAD"]).unwrap()
}

fn package(version: &str) -> String {
    serde_json::json!({"schema_version":1,"id":"mod","game":"test-game","name":"Mod",
        "version":version,"archive_format":"zip","sha256":"0".repeat(64),
        "source":{"kind":"manual","filename":"mod.zip"}})
    .to_string()
}

#[test]
fn explicit_sync_tracks_commits_and_retains_previous_on_failure() {
    let remote = tempfile::tempdir().unwrap();
    git(remote.path(), &["init", "--initial-branch=fixture"]).unwrap();
    fs::write(
        remote.path().join("bucket.json"),
        r#"{"schema_version":1,"name":"community","title":"Community"}"#,
    )
    .unwrap();
    fs::create_dir_all(remote.path().join("games/test-game")).unwrap();
    let manifest = remote.path().join("games/test-game/mod.json");
    fs::write(&manifest, package("1")).unwrap();
    let first = commit(remote.path(), "first");
    let state = tempfile::tempdir().unwrap();
    let store = BucketStore::new(state.path());
    let source = GitCatalogSource {
        remote: remote.path().to_string_lossy().into_owned(),
        reference: "fixture".into(),
    };
    assert!(store.git_revision("community").unwrap().is_none());
    assert_eq!(
        store.sync_git("community", Some(source)).unwrap().commit,
        first
    );
    let registration = state.path().join("buckets/registrations/community.json");
    let old_registration = fs::read(&registration).unwrap();
    fs::write(&manifest, package("2")).unwrap();
    let second = commit(remote.path(), "second");
    assert_eq!(
        store.git_revision("community").unwrap().unwrap().commit,
        first
    );
    assert_eq!(store.sync_git("community", None).unwrap().commit, second);
    assert_ne!(fs::read(&registration).unwrap(), old_registration);
    let usable = fs::read(&registration).unwrap();
    let pointer: serde_json::Value = serde_json::from_slice(&usable).unwrap();
    let published = Path::new(pointer["path"].as_str().unwrap());
    assert!(
        fs::read_to_string(published.join("games/test-game/mod.json"))
            .unwrap()
            .contains("\"2\"")
    );
    fs::write(remote.path().join("games/test-game/module.json"), "{}").unwrap();
    commit(remote.path(), "invalid module");
    assert!(store.sync_git("community", None).is_err());
    assert_eq!(fs::read(&registration).unwrap(), usable);
    fs::remove_file(remote.path().join("games/test-game/module.json")).unwrap();
    fs::write(&manifest, "invalid JSON").unwrap();
    commit(remote.path(), "invalid metadata");
    assert!(store.sync_git("community", None).is_err());
    assert_eq!(fs::read(&registration).unwrap(), usable);
    git(remote.path(), &["update-ref", "-d", "refs/heads/fixture"]).unwrap();
    assert!(store.sync_git("community", None).is_err());
    assert_eq!(fs::read(&registration).unwrap(), usable);
    fs::rename(
        remote.path().join(".git"),
        remote.path().join("unavailable"),
    )
    .unwrap();
    assert!(store.sync_git("community", None).is_err());
    assert_eq!(fs::read(&registration).unwrap(), usable);
    assert!(published.join("games/test-game/mod.json").is_file());
}

#[test]
fn rejects_reserved_names_and_unsafe_sources_without_fetching() {
    let state = tempfile::tempdir().unwrap();
    let store = BucketStore::new(state.path());
    for remote in [
        "ext::evil",
        "http://example.invalid/repo",
        "--upload-pack=evil",
        "https://user@example.invalid/repo",
    ] {
        let source = GitCatalogSource {
            remote: remote.into(),
            reference: "HEAD".into(),
        };
        assert!(store.sync_git("community", Some(source)).is_err());
    }
    assert!(store.sync_git("main", None).is_err());
}
