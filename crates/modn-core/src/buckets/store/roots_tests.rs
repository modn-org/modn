use super::BucketStore;
use crate::manager_roots::ManagerRoots;
use std::fs;

#[test]
fn official_copy_comes_from_data_while_registrations_come_from_profile() {
    let temp = tempfile::tempdir().expect("fixture");
    let data = temp.path().join("data");
    let profile = temp.path().join("profile");
    let main = data.join("buckets/main");
    let local = temp.path().join("local");
    fs::create_dir_all(&main).expect("main directory");
    fs::create_dir_all(&local).expect("local bucket directory");
    fs::write(
        main.join("bucket.json"),
        r#"{"schema_version":2,"name":"main","title":"Main"}"#,
    )
    .expect("main metadata");
    fs::write(
        local.join("bucket.json"),
        r#"{"schema_version":2,"name":"community","title":"Community"}"#,
    )
    .expect("local metadata");

    let roots = ManagerRoots::new(&data, Some(&profile));
    let store = BucketStore::new(&roots);
    let (official, _) = store.official().expect("official main bucket");
    assert_eq!(official.path.as_deref(), Some(main.as_path()));
    store.add("community", &local).expect("register bucket");
    assert!(registration_path(roots.profile(), "community").is_file());
    assert!(!registration_path(roots.data(), "community").is_file());
    let local = local.canonicalize().expect("local bucket path");
    let listed = store.list().expect("list buckets");
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].path.as_deref(), Some(main.as_path()));
    assert_eq!(listed[1].path.as_deref(), Some(local.as_path()));
}

fn registration_path(profile: &std::path::Path, bucket: &str) -> std::path::PathBuf {
    profile
        .join("buckets/registrations")
        .join(format!("{bucket}.json"))
}
