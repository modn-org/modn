#![allow(
    dead_code,
    reason = "shared fixture exposes helpers for different test crates"
)]

pub fn copy_tree(source: &std::path::Path, target: &std::path::Path) {
    std::fs::create_dir_all(target).expect("fixture invariant");
    for entry in std::fs::read_dir(source).expect("fixture invariant") {
        let entry = entry.expect("fixture invariant");
        if entry.file_name() == ".git" {
            continue;
        }
        let target = target.join(entry.file_name());
        if entry.file_type().expect("fixture invariant").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).expect("fixture invariant");
        }
    }
}

pub fn copy_main_module(source: &std::path::Path, target: &std::path::Path) {
    copy_tree(source, target);
    let manifest_path = target.join("module.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).expect("fixture invariant"))
            .expect("fixture invariant");
    manifest["api_version"] = serde_json::json!(3);
    std::fs::write(
        &manifest_path,
        serde_json::to_vec(&manifest).expect("fixture invariant"),
    )
    .expect("fixture invariant");
}

pub fn roots(profile: &std::path::Path) -> modn_core::manager_roots::ManagerRoots {
    modn_core::manager_roots::ManagerRoots::new(
        &modn_core::data_dir::data_directory().expect("fixture data directory"),
        Some(profile),
    )
}

pub fn seeded<T>(action: impl FnOnce() -> T) -> T {
    let temp = tempfile::tempdir().expect("fixture invariant");
    let target = temp.path().join("buckets/main");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../buckets/main");
    copy_tree(&root, &target);
    modn_core::data_dir::with_data_directory(temp.path(), action)
}

pub fn isolated_profile<T>(action: impl FnOnce(&std::path::Path) -> T) -> T {
    let profile = tempfile::tempdir().expect("fixture invariant");
    modn_core::data_dir::with_profile_directory(profile.path(), || action(profile.path()))
}
