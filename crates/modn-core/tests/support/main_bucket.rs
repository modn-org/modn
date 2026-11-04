pub fn seeded<T>(action: impl FnOnce() -> T) -> T {
    fn copy(source: &std::path::Path, target: &std::path::Path) {
        std::fs::create_dir_all(target).expect("fixture invariant");
        for entry in std::fs::read_dir(source).expect("fixture invariant") {
            let entry = entry.expect("fixture invariant");
            let target = target.join(entry.file_name());
            if entry.file_type().expect("fixture invariant").is_dir() {
                copy(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).expect("fixture invariant");
            }
        }
    }
    let temp = tempfile::tempdir().expect("fixture invariant");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../buckets/main");
    copy(&root, &temp.path().join("buckets/main"));
    modn_core::data_dir::with_data_directory(temp.path(), action)
}
