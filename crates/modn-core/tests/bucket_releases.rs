#[path = "support/main_bucket.rs"]
mod main_bucket;
use modn_core::buckets::BucketStore;
use serde_json::json;
use std::fs;
use std::path::Path;

fn write_release(bucket: &Path, file: &str, id: &str, version: &str) {
    let manifest = json!({
        "schema_version": 1, "id": id, "game": "test-game", "name": id,
        "version": version, "archive_format": "zip", "sha256": "a".repeat(64),
        "source": {"kind": "manual", "filename": format!("{id}-{version}.zip")}
    });
    fs::write(
        bucket.join("games/test-game").join(file),
        serde_json::to_vec(&manifest).expect("JSON"),
    )
    .expect("manifest");
}

fn bucket(temp: &Path, name: &str) -> std::path::PathBuf {
    let root = temp.join(name);
    fs::create_dir_all(root.join("games/test-game")).expect("bucket");
    fs::write(
        root.join("bucket.json"),
        serde_json::to_vec(&json!({"schema_version": 1, "name": name, "title": "Fixture"}))
            .expect("JSON"),
    )
    .expect("metadata");
    root
}

#[test]
fn other_releases_are_listed_only_for_their_mod() {
    main_bucket::seeded(|| {
        let temp = tempfile::tempdir().expect("temp");
        let root = bucket(temp.path(), "local");
        write_release(&root, "foo.json", "foo", "1.1");
        write_release(&root, "foo@1.0.json", "foo", "1.0");
        write_release(&root, "foo@0.9.json", "foo", "0.9");
        let store = BucketStore::new(&temp.path().join("state"));
        store.add("local", &root).expect("register");

        let current: Vec<String> = store
            .packages(Some("test-game"))
            .expect("packages")
            .into_iter()
            .filter(|p| p.bucket == "local")
            .map(|p| p.package.version)
            .collect();
        assert_eq!(current, ["1.1"]);
        let versions: Vec<String> = store
            .releases("local", "test-game", "foo")
            .expect("releases")
            .into_iter()
            .map(|p| p.package.version)
            .collect();
        assert_eq!(versions, ["1.1", "0.9", "1.0"]);
        assert_eq!(
            store
                .releases("main", "cyberpunk-2077", "red4ext")
                .expect("main")
                .len(),
            1
        );
    });
}

#[test]
fn malformed_release_files_are_rejected() {
    main_bucket::seeded(|| {
        for (file, id, version, message) in [
            ("foo@2.0.json", "foo", "1.9", "disagrees with its version"),
            ("bar@1.0.json", "bar", "1.0", "has no current release"),
            ("foo@1.1.json", "foo", "1.1", "duplicate release"),
            ("foo@1.0.json", "other", "1.0", "identity disagrees"),
        ] {
            let temp = tempfile::tempdir().expect("temp");
            let root = bucket(temp.path(), "local");
            write_release(&root, "foo.json", "foo", "1.1");
            write_release(&root, file, id, version);
            let error = BucketStore::new(&temp.path().join("state"))
                .add("local", &root)
                .expect_err(file);
            assert!(format!("{error:#}").contains(message), "{file}: {error:#}");
        }
    });
}
