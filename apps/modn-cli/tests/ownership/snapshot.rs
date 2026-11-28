//! Filesystem snapshots and JSON helpers for the ownership model.

use proptest::test_runner::TestCaseError;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

pub(crate) type Snapshot = BTreeMap<PathBuf, Option<Vec<u8>>>;

pub(crate) fn snapshot(root: &Path) -> Result<Snapshot, TestCaseError> {
    let mut entries = BTreeMap::new();
    if fs::symlink_metadata(root).is_err() {
        return Ok(entries);
    }
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)? {
            let path = entry?.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|err| TestCaseError::fail(err.to_string()))?
                .to_path_buf();
            if fs::symlink_metadata(&path)?.is_dir() {
                pending.push(path);
                entries.insert(relative, None);
            } else {
                entries.insert(relative, Some(fs::read(&path)?));
            }
        }
    }
    Ok(entries)
}

pub(crate) fn files(snapshot: &Snapshot) -> BTreeMap<&PathBuf, &Vec<u8>> {
    snapshot
        .iter()
        .filter_map(|(path, bytes)| bytes.as_ref().map(|bytes| (path, bytes)))
        .collect()
}

pub(crate) fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

pub(crate) fn path_key(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        text.to_ascii_lowercase()
    } else {
        text
    }
}

pub(crate) fn json(output: &Output) -> Result<Value, TestCaseError> {
    serde_json::from_slice(&output.stdout).map_err(|err| TestCaseError::fail(err.to_string()))
}
