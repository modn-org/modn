use super::resolve;
use modn_core::buckets::BucketStore;
use serde_json::json;
use std::fs;
use std::path::Path;

fn manifest(root: &Path, id: &str, digest: &str, dependencies: &[&str]) {
    fs::write(
        root.join(format!("mods/test-game/{id}.json")),
        serde_json::to_vec(&json!({
            "schema_version": 2, "id": id, "game": "test-game", "name": id,
            "version": "1.0", "archive_format": "zip", "blake3": digest,
            "sources": [{"kind": "manual", "filename": format!("{id}.zip")}],
            "dependencies": dependencies,
        }))
        .expect("manifest JSON"),
    )
    .expect("write manifest");
}

#[test]
fn selected_release_keeps_dependency_pins_when_bucket_changes() {
    let temp = tempfile::tempdir().expect("fixture");
    let data = temp.path().join("data");
    let official = data.join("buckets/main");
    let local = temp.path().join("local");
    fs::create_dir_all(&official).expect("official bucket");
    fs::create_dir_all(local.join("mods/test-game")).expect("local bucket");
    for (path, name) in [(&official, "main"), (&local, "local")] {
        fs::write(
            path.join("bucket.json"),
            serde_json::to_vec(&json!({
                "schema_version": 2, "name": name, "title": "Fixture",
            }))
            .expect("bucket JSON"),
        )
        .expect("bucket metadata");
    }
    let original = "a".repeat(64);
    manifest(&local, "base", &original, &[]);
    manifest(&local, "top", &"c".repeat(64), &["base"]);
    modn_core::data_dir::with_data_directory(&data, || {
        let store = BucketStore::new(&temp.path().join("state"));
        store.add("local", &local).expect("register bucket");
        let catalog = store.snapshot(None).expect("snapshot");
        let selected = resolve("local/test-game/top", &catalog, true).expect("selection");

        manifest(&local, "base", &"b".repeat(64), &[]);
        let release = catalog.release(&selected).expect("release evidence");

        assert_eq!(release.dependencies.len(), 1);
        assert_eq!(release.dependencies[0].archive_sha256, original);
    });
}
