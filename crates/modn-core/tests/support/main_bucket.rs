pub fn seeded<T>(action: impl FnOnce() -> T) -> T {
    fn copy(source: &std::path::Path, target: &std::path::Path) {
        std::fs::create_dir_all(target).expect("fixture invariant");
        for entry in std::fs::read_dir(source).expect("fixture invariant") {
            let entry = entry.expect("fixture invariant");
            if entry.file_name() == ".git" {
                continue;
            }
            let target = target.join(entry.file_name());
            if entry.file_type().expect("fixture invariant").is_dir() {
                copy(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).expect("fixture invariant");
            }
        }
    }
    let temp = tempfile::tempdir().expect("fixture invariant");
    let target = temp.path().join("buckets/main");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../buckets/main");
    copy(&root, &target);
    to_contract_v2(&target);
    modn_core::data_dir::with_data_directory(temp.path(), action)
}

/// Rewrite a checked-in v1 main copy as a contract v2 fixture.
///
/// The real submodule stays v1 until a maintainer rehashes it. Test fixtures
/// carry the historical digest as the v2 pin because no archive is available.
pub fn to_contract_v2(root: &std::path::Path) {
    let metadata_path = root.join("bucket.json");
    let mut metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&metadata_path).expect("bucket.json"))
            .expect("bucket metadata");
    metadata["schema_version"] = serde_json::json!(2);
    std::fs::write(&metadata_path, serde_json::to_vec(&metadata).expect("JSON"))
        .expect("bucket metadata");

    let games = root.join("games");
    for game in std::fs::read_dir(&games).expect("games") {
        let game = game.expect("game");
        let name = game.file_name();
        let mods = root.join("mods").join(&name);
        for entry in std::fs::read_dir(game.path()).expect("game entries") {
            let entry = entry.expect("game entry");
            let path = entry.path();
            if path.file_name().is_some_and(|name| name == "module.json")
                || path.extension().is_none_or(|extension| extension != "json")
            {
                continue;
            }
            let mut record: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).expect("record")).expect("record");
            record["schema_version"] = serde_json::json!(2);
            if let Some(source) = record.as_object_mut().and_then(|r| r.remove("source")) {
                record["sources"] = serde_json::json!([source]);
            }
            if let Some(pin) = record.as_object_mut().and_then(|r| r.remove("sha256")) {
                record["blake3"] = pin;
            }
            std::fs::create_dir_all(&mods).expect("mods");
            std::fs::write(
                mods.join(entry.file_name()),
                serde_json::to_vec(&record).expect("JSON"),
            )
            .expect("record");
            std::fs::remove_file(&path).expect("remove v1 record");
        }
    }
}
