use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub fn snapshot(root: &Path) -> BTreeMap<String, String> {
    fn walk(root: &Path, dir: &Path, files: &mut BTreeMap<String, String>) {
        for entry in fs::read_dir(dir).expect("read dir") {
            let path = entry.expect("entry").path();
            let relative = path
                .strip_prefix(root)
                .expect("prefix")
                .to_string_lossy()
                .replace('\\', "/");
            if path.is_dir() {
                files.insert(format!("{relative}/"), DIR.to_owned());
                walk(root, &path, files);
            } else {
                let digest = format!("{:x}", Sha256::digest(fs::read(&path).expect("file")));
                files.insert(relative, digest);
            }
        }
    }
    let mut files = BTreeMap::new();
    walk(root, root, &mut files);
    files
}

pub const DIR: &str = "DIR";

pub fn file_count(snapshot: &BTreeMap<String, String>) -> usize {
    snapshot.values().filter(|value| *value != DIR).count()
}

pub fn sha(path: &Path) -> String {
    format!("{:x}", Sha256::digest(fs::read(path).expect("archive")))
}
