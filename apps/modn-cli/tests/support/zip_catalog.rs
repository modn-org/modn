#![allow(
    dead_code,
    reason = "shared fixture exposes helpers for different test crates"
)]
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
/// Copy every file of one catalog fixture into `target`.
pub fn copy_catalog(source: &Path, target: &Path) {
    fs::create_dir_all(target).expect("catalog directory");
    for entry in fs::read_dir(source).expect("catalog entries") {
        let entry = entry.expect("catalog entry");
        if entry.file_name() == ".git" {
            continue;
        }
        let target = target.join(entry.file_name());
        if entry.file_type().expect("catalog entry type").is_dir() {
            copy_catalog(&entry.path(), &target);
        } else {
            fs::write(&target, fs::read(entry.path()).expect("catalog bytes"))
                .expect("catalog file");
        }
    }
}
/// Pack one catalog directory into a ZIP with sorted entry names.
pub fn pack(source: &Path, zip: &Path) {
    fn files(root: &Path, current: &Path, output: &mut Vec<(String, Vec<u8>)>) {
        for entry in fs::read_dir(current).expect("fixture directory") {
            let path = entry.expect("fixture entry").path();
            if path.is_dir() {
                files(root, &path, output);
            } else {
                output.push((
                    path.strip_prefix(root)
                        .expect("fixture relative path")
                        .to_str()
                        .expect("fixture UTF-8 path")
                        .replace('\\', "/"),
                    fs::read(path).expect("fixture file"),
                ));
            }
        }
    }
    let mut entries = Vec::new();
    files(source, source, &mut entries);
    entries.sort();
    let mut writer = zip::ZipWriter::new(fs::File::create(zip).expect("fixture ZIP"));
    for (name, bytes) in entries {
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .expect("fixture ZIP entry");
        writer.write_all(&bytes).expect("fixture ZIP bytes");
    }
    writer.finish().expect("fixture ZIP complete");
}
/// Copy one signed fixture catalog and pack it as a ZIP.
pub fn fixture_zip(root: &Path, fixture: &str) -> PathBuf {
    let directory = root.join(format!("{fixture}-catalog"));
    copy_catalog(&catalog_fixture(fixture), &directory);
    let zip = root.join(format!("{fixture}.zip"));
    pack(&directory, &zip);
    zip
}
pub fn catalog_fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/signed-catalog")
        .join(name)
}
/// Every regular file under `root` with relative path and bytes.
pub fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
    fn collect(root: &Path, directory: &Path, output: &mut Vec<(String, Vec<u8>)>) {
        for entry in fs::read_dir(directory).expect("snapshot entries") {
            let entry = entry.expect("snapshot entry");
            let path = entry.path();
            if entry.file_type().expect("snapshot type").is_dir() {
                collect(root, &path, output);
            } else {
                output.push((
                    path.strip_prefix(root)
                        .expect("snapshot path")
                        .to_string_lossy()
                        .replace('\\', "/"),
                    fs::read(&path).expect("snapshot bytes"),
                ));
            }
        }
    }
    let mut output = Vec::new();
    collect(root, root, &mut output);
    output.sort();
    output
}
/// Entry names directly under `root`.
pub fn listing(root: &Path) -> Vec<String> {
    fs::read_dir(root)
        .map(|entries| {
            let mut names: Vec<String> = entries
                .map(|entry| {
                    entry
                        .expect("listing entry")
                        .file_name()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect();
            names.sort();
            names
        })
        .unwrap_or_default()
}
