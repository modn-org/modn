#![allow(
    dead_code,
    reason = "shared fixture exposes helpers for different test crates"
)]
use crate::catalog_authoring::{
    copy_tree, fingerprint, json, path_text, prepare_catalog, run, stderr,
};
use crate::zip_catalog::pack;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

pub const MIRROR: &str = "https://mirror.example/catalog.zip";
pub const SECOND_MIRROR: &str = "https://second.example/catalog.zip";
pub const NEW_PRIMARY: &str = "https://new.example/catalog.zip";

pub fn keygen(keys: &Path, name: &str) -> String {
    let generated = run(&[
        "--json",
        "catalog",
        "keygen",
        "--out",
        path_text(keys),
        "--name",
        name,
    ]);
    assert!(
        generated.status.success(),
        "keygen failed: {}",
        stderr(&generated)
    );
    fingerprint(&generated)
}

pub fn signed_catalog(
    root: &Path,
    name: &str,
    keys: &Path,
    key_name: &str,
    extra: &[(&str, &str)],
) -> PathBuf {
    let catalog = prepare_catalog(root, name);
    for (file, body) in extra {
        fs::write(catalog.join(file), body).expect("catalog metadata");
    }
    let signed = run(&[
        "--json",
        "catalog",
        "sign",
        path_text(&catalog),
        "--key",
        path_text(&keys.join(format!("{key_name}.key"))),
    ]);
    assert!(signed.status.success(), "sign failed: {}", stderr(&signed));
    catalog
}

pub fn pack_catalog(catalog: &Path, zip: &Path) -> PathBuf {
    pack(catalog, zip);
    zip.to_path_buf()
}

pub fn mirrors_json(urls: &[&str]) -> String {
    let mirrors: Vec<Value> = urls
        .iter()
        .map(|url| serde_json::json!({"kind":"zip","url":url}))
        .collect();
    serde_json::json!({"schema_version":1,"mirrors":mirrors}).to_string()
}

pub fn relocation_json(url: &str) -> String {
    serde_json::json!({"schema_version":1,"primary":{"kind":"zip","url":url}}).to_string()
}

pub fn run_state(state: &Path, args: &[&str]) -> Output {
    let mut full = vec!["--state-dir", path_text(state)];
    full.extend_from_slice(args);
    run(&full)
}

pub fn run_state_json(state: &Path, args: &[&str]) -> Output {
    let mut full = vec!["--state-dir", path_text(state), "--json"];
    full.extend_from_slice(args);
    run(&full)
}

pub fn listed_entry(state: &Path, name: &str) -> Value {
    let output = run_state_json(state, &["bucket", "list"]);
    assert!(output.status.success(), "list failed: {}", stderr(&output));
    let value = json(&output);
    value
        .as_array()
        .expect("bucket list array")
        .iter()
        .find(|entry| entry["name"] == name)
        .cloned()
        .expect("registered bucket")
}

pub fn registration(state: &Path, name: &str) -> Value {
    let bytes = fs::read(state.join(format!("buckets/registrations/{name}.json")))
        .expect("registration file");
    serde_json::from_slice(&bytes).expect("registration JSON")
}

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

pub fn published_path(state: &Path, name: &str) -> PathBuf {
    PathBuf::from(
        registration(state, name)["path"]
            .as_str()
            .expect("published path"),
    )
}

pub fn refresh_copy(source: &Path, target: &Path) {
    if target.exists() {
        fs::remove_dir_all(target).expect("remove old catalog");
    }
    copy_tree(source, target);
}
