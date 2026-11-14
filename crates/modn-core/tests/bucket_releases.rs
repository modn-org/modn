#[path = "support/main_bucket.rs"]
mod main_bucket;
use modn_core::buckets::BucketStore;
use serde_json::json;
use std::fs;
use std::path::Path;

fn write_release(bucket: &Path, file: &str, id: &str, version: &str) {
    let manifest = json!({
        "schema_version": 2, "id": id, "game": "test-game", "name": id,
        "version": version, "archive_format": "zip", "blake3": "a".repeat(64),
        "sources": [{"kind": "manual", "filename": format!("{id}-{version}.zip")}]
    });
    fs::write(
        bucket.join("mods/test-game").join(file),
        serde_json::to_vec(&manifest).expect("JSON"),
    )
    .expect("manifest");
}

fn bucket(temp: &Path, name: &str) -> std::path::PathBuf {
    let root = temp.join(name);
    fs::create_dir_all(root.join("mods/test-game")).expect("bucket");
    fs::write(
        root.join("bucket.json"),
        serde_json::to_vec(&json!({"schema_version": 2, "name": name, "title": "Fixture"}))
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

#[test]
fn contract_v1_bucket_json_is_refused_at_load_without_writing_state() {
    main_bucket::seeded(|| {
        let temp = tempfile::tempdir().expect("temp");
        let root = bucket(temp.path(), "local");
        write_release(&root, "foo.json", "foo", "1.0");
        let store = BucketStore::new(&temp.path().join("state"));
        store.add("local", &root).expect("register");
        let registration = temp.path().join("state/buckets/registrations/local.json");
        let before = fs::read(&registration).expect("registration");
        fs::write(
            root.join("bucket.json"),
            r#"{"schema_version":1,"name":"local","title":"Fixture"}"#,
        )
        .expect("v1 metadata");

        let error = store.packages(Some("test-game")).expect_err("v1 load");
        let message = format!("{error:#}");
        assert!(message.contains("schema_version 1"), "{message}");
        assert!(message.contains("bucket contract v2"), "{message}");
        assert_eq!(fs::read(&registration).expect("registration"), before);
    });
}

#[test]
fn record_file_names_are_lowercase_and_case_collisions_are_refused() {
    main_bucket::seeded(|| {
        let temp = tempfile::tempdir().expect("temp");
        let root = bucket(temp.path(), "local");
        write_release(&root, "foo.json", "foo", "1.0");
        fs::rename(
            root.join("mods/test-game/foo.json"),
            root.join("mods/test-game/Foo.json"),
        )
        .expect("rename record");
        let error = BucketStore::new(&temp.path().join("state"))
            .add("local", &root)
            .expect_err("mixed-case record name");
        assert!(
            format!("{error:#}").contains("invalid package filename"),
            "{error:#}"
        );
    });
}

#[cfg(unix)]
#[test]
fn case_colliding_records_are_refused() {
    main_bucket::seeded(|| {
        let temp = tempfile::tempdir().expect("temp");
        let root = bucket(temp.path(), "local");
        write_release(&root, "foo.json", "foo", "1.0");
        write_release(&root, "FOO.json", "foo", "1.0");
        let error = BucketStore::new(&temp.path().join("state"))
            .add("local", &root)
            .expect_err("case collision");
        assert!(
            format!("{error:#}").contains("invalid package filename"),
            "{error:#}"
        );
    });
}

#[cfg(unix)]
#[test]
fn linked_mods_records_are_refused() {
    use std::os::unix::fs::symlink;
    main_bucket::seeded(|| {
        let temp = tempfile::tempdir().expect("temp");
        let root = bucket(temp.path(), "local");
        write_release(&root, "foo.json", "foo", "1.0");
        let record = root.join("mods/test-game/foo.json");
        let outside = temp.path().join("outside.json");
        fs::rename(&record, &outside).expect("move record");
        symlink(&outside, &record).expect("link record");

        let error = BucketStore::new(&temp.path().join("state"))
            .add("local", &root)
            .expect_err("linked record");
        assert!(
            format!("{error:#}").contains("unexpected bucket package entry"),
            "{error:#}"
        );

        fs::remove_file(&record).expect("remove link");
        fs::remove_dir_all(root.join("mods")).expect("remove mods");
        symlink(temp.path().join("mods-target"), root.join("mods")).expect("link mods");
        fs::create_dir_all(temp.path().join("mods-target/test-game")).expect("target");
        let error = BucketStore::new(&temp.path().join("state"))
            .add("local", &root)
            .expect_err("linked mods directory");
        assert!(
            format!("{error:#}").contains("unsafe bucket directory"),
            "{error:#}"
        );
    });
}

#[test]
fn snapshot_keeps_selection_and_historical_versions_after_bucket_edits() {
    main_bucket::seeded(|| {
        let temp = tempfile::tempdir().expect("temp");
        let root = bucket(temp.path(), "local");
        write_release(&root, "foo.json", "foo", "1.1");
        write_release(&root, "foo@1.0.json", "foo", "1.0");
        let store = BucketStore::new(&temp.path().join("state"));
        store.add("local", &root).expect("register");
        let snapshot = store.snapshot(Some("test-game")).expect("snapshot");

        write_release(&root, "foo.json", "foo", "2.0");
        fs::write(root.join("mods/test-game/foo@1.0.json"), b"invalid JSON")
            .expect("edit historical manifest");

        for query in ["foo", "test-game/foo", "local/test-game/foo"] {
            let candidates = snapshot.candidates(query).expect("resolve snapshot");
            assert_eq!(candidates.len(), 1);
            assert_eq!(candidates[0].package.version, "1.1");
        }
        let versions: Vec<String> = snapshot
            .releases("local", "test-game", "foo")
            .expect("retained releases")
            .into_iter()
            .map(|release| release.package.version)
            .collect();
        assert_eq!(versions, ["1.1", "1.0"]);
        assert!(snapshot
            .candidates("main/test-game/foo")
            .unwrap()
            .is_empty());
        assert!(snapshot.candidates("/foo").is_err());
        assert!(snapshot.releases("other", "test-game", "foo").is_err());
        assert!(store.snapshot(Some("test-game")).is_err());
    });
}

#[test]
fn a_package_entry_that_is_not_a_regular_file_is_refused_by_name() {
    main_bucket::seeded(|| {
        let temp = tempfile::tempdir().expect("temp");
        let root = bucket(temp.path(), "local");
        fs::create_dir(root.join("mods/test-game/foo.json")).expect("directory entry");
        let state = temp.path().join("state");
        let error = BucketStore::new(&state)
            .add("local", &root)
            .expect_err("special package entry");
        let message = format!("{error:#}");
        assert!(message.contains("regular file"), "{message}");
        assert!(message.contains("foo.json"), "{message}");
        assert!(!state.join("buckets/registrations/local.json").exists());
    });
}
