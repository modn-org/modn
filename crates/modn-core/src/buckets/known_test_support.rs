//! Shared fixtures for the `known-buckets.json` unit tests.

use crate::buckets::BucketStore;
use minisign::KeyPair;
use std::fs;
use std::io::{self, Cursor, Write};
use std::path::{Path, PathBuf};

pub(super) const VALID: &str = r#"{
  "schema_version": 1,
  "buckets": {
    "zeta": {
      "kind": "zip",
      "source": "https://mirror.example/zeta.zip",
      "fingerprint": "abcdef0123456789",
      "description": "Zeta bucket."
    },
    "alpha": {
      "kind": "git",
      "source": "https://github.com/owner/alpha-bucket",
      "revision": "main",
      "fingerprint": "0123456789ABCDEF",
      "description": "Alpha bucket."
    }
  }
}"#;

pub(super) fn entry(
    kind: &str,
    source: &str,
    revision: Option<&str>,
    fingerprint: &str,
    description: &str,
) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert("kind".into(), kind.into());
    map.insert("source".into(), source.into());
    if let Some(revision) = revision {
        map.insert("revision".into(), revision.into());
    }
    map.insert("fingerprint".into(), fingerprint.into());
    map.insert("description".into(), description.into());
    serde_json::Value::Object(map)
}

pub(super) fn file_with(name: &str, entry: serde_json::Value) -> String {
    let mut buckets = serde_json::Map::new();
    buckets.insert(name.to_owned(), entry);
    serde_json::json!({"schema_version": 1, "buckets": buckets}).to_string()
}

pub(super) fn git_entry() -> serde_json::Value {
    entry(
        "git",
        "https://github.com/owner/bucket",
        Some("main"),
        "0e250ad52a3ba2aa",
        "A community bucket.",
    )
}

pub(super) fn seed_main(root: &Path, known: Option<&str>) {
    let main = root.join("buckets/main");
    fs::create_dir_all(&main).expect("fixture main");
    fs::write(
        main.join("bucket.json"),
        r#"{"schema_version":2,"name":"main","title":"Main"}"#,
    )
    .expect("fixture metadata");
    if let Some(known) = known {
        fs::write(main.join(super::KNOWN_BUCKETS_FILE), known).expect("fixture known buckets");
    }
}

fn pack(tree: &Path, zip: &Path) {
    fn files(current: &Path, root: &Path, output: &mut Vec<(String, Vec<u8>)>) {
        for entry in fs::read_dir(current).expect("fixture entries") {
            let path = entry.expect("fixture entry").path();
            if path.is_dir() {
                files(&path, root, output);
            } else {
                output.push((
                    path.strip_prefix(root)
                        .expect("fixture relative path")
                        .to_str()
                        .expect("fixture UTF-8 path")
                        .replace('\\', "/"),
                    fs::read(&path).expect("fixture file"),
                ));
            }
        }
    }
    let mut entries = Vec::new();
    files(tree, tree, &mut entries);
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

/// Sign one `main` tree and run the real update path.
pub(super) fn install(store: &BucketStore, tree: &Path, parent: &Path) -> anyhow::Result<()> {
    let zip = parent.join("main.zip");
    pack(tree, &zip);
    let key = KeyPair::generate_unencrypted_keypair().expect("fixture key");
    let public = key.pk.to_box().expect("fixture key box").into_string();
    let statement = crate::buckets::catalog_statement(tree).expect("fixture statement");
    let signature = minisign::sign(
        Some(&key.pk),
        &key.sk,
        Cursor::new(statement.as_bytes()),
        Some("known buckets fixture"),
        Some("known buckets fixture signature"),
    )
    .expect("fixture signature")
    .into_string();
    store
        .update_main(
            &public,
            || -> anyhow::Result<fs::File> { Ok(fs::File::open(&zip)?) },
            || -> anyhow::Result<io::Cursor<Vec<u8>>> {
                Ok(io::Cursor::new(signature.clone().into_bytes()))
            },
        )
        .map(|_| ())
}

pub(super) fn tree_with(parent: &Path, known: Option<&str>) -> PathBuf {
    let tree = parent.join("tree");
    fs::create_dir_all(&tree).expect("fixture tree");
    fs::write(
        tree.join("bucket.json"),
        r#"{"schema_version":2,"name":"main","title":"Main"}"#,
    )
    .expect("fixture metadata");
    if let Some(known) = known {
        fs::write(tree.join(super::KNOWN_BUCKETS_FILE), known).expect("fixture known buckets");
    }
    tree
}
