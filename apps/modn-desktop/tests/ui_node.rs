//! Runs the desktop UI behaviour tests with the Node test runner.
//!
//! The UI ships as plain ES modules, so the behaviour tests under `tests/js`
//! import the real modules and stub `window`, `document` and timers. Node is a
//! developer tool, not a build requirement: a machine without `node` on PATH
//! prints a skip message and passes.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

/// Lists every `*.test.mjs` file under `directory`, sorted.
///
/// Some Node releases treat a directory argument to `node --test` as a module
/// path, so the test passes the explicit file list instead.
fn test_files(directory: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![directory.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(&next).expect("read a UI test directory") {
            let path = entry.expect("read a UI test directory entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.to_string_lossy().ends_with(".test.mjs") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

#[test]
fn ui_behaviour_tests_pass() {
    let directory = Path::new(ROOT).join("tests").join("js");
    let files = test_files(&directory);
    assert!(
        !files.is_empty(),
        "the UI behaviour tests must live in {directory:?}"
    );
    let version = match Command::new("node").arg("--version").output() {
        Ok(output) => String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("skipping UI behaviour tests: node is not on PATH");
            return;
        }
        Err(error) => panic!("cannot run node for the UI behaviour tests: {error}"),
    };
    let output = Command::new("node")
        .arg("--test")
        .args(&files)
        .output()
        .expect("run node --test");
    assert!(
        output.status.success(),
        "node --test failed (node {version}, {} files)\nstdout:\n{}\nstderr:\n{}",
        files.len(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
