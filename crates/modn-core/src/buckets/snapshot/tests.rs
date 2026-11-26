use super::BucketSnapshot;
use crate::acquisition::ReleaseDigest;
use crate::buckets::BucketPackage;
use serde_json::json;

fn package(id: &str, blake3: Option<&str>, dependencies: &[&str]) -> BucketPackage {
    let source = match blake3 {
        Some(_) => json!({"kind": "manual", "filename": format!("{id}.zip")}),
        None => json!({
            "kind": "git",
            "repository": "owner/project",
            "commit": "a".repeat(40)
        }),
    };
    let mut value = json!({
        "schema_version": 2,
        "id": id,
        "game": "test-game",
        "name": id,
        "version": "1.0",
        "sources": [source],
        "dependencies": dependencies,
    });
    if let Some(blake3) = blake3 {
        value["archive_format"] = json!("zip");
        value["blake3"] = json!(blake3);
    }
    BucketPackage {
        bucket: "main".to_owned(),
        package: serde_json::from_value(value).expect("package manifest"),
    }
}

#[test]
fn an_unpinned_git_dependency_is_unknown_in_the_snapshot() {
    let selected = package("top", Some(&"b".repeat(64)), &["base"]);
    let dependency = package("base", None, &[]);
    let snapshot = BucketSnapshot::new(vec![selected.clone(), dependency], Vec::new());

    let release = snapshot.release(&selected).expect("release evidence");

    assert_eq!(release.dependencies.len(), 1);
    assert_eq!(release.dependencies[0].reference.mod_id, "base");
    assert_eq!(
        release.dependencies[0].archive_blake3,
        ReleaseDigest::Unknown
    );
}

#[test]
fn a_pinned_dependency_keeps_the_catalog_pin() {
    let pin = "a".repeat(64);
    let selected = package("top", Some(&"b".repeat(64)), &["base"]);
    let dependency = package("base", Some(&pin), &[]);
    let snapshot = BucketSnapshot::new(vec![selected.clone(), dependency], Vec::new());

    let release = snapshot.release(&selected).expect("release evidence");

    assert_eq!(
        release.dependencies[0].archive_blake3,
        ReleaseDigest::pinned(&pin).expect("pin")
    );
}
