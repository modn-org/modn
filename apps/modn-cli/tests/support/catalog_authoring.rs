#![allow(
    dead_code,
    reason = "each integration-test crate uses a different subset of this fixture"
)]
use crate::main_bucket;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Run the `modn` binary with an isolated data directory.
pub fn run(args: &[&str]) -> Output {
    main_bucket::command(env!("CARGO_BIN_EXE_modn"))
        .args(args)
        .output()
        .expect("test fixture invariant")
}

/// Run the `modn` binary with extra environment variables and an isolated data directory.
pub fn run_with(args: &[&str], variables: &[(&str, &str)]) -> Output {
    let mut command = main_bucket::command(env!("CARGO_BIN_EXE_modn"));
    for (name, value) in variables {
        command.env(name, value);
    }
    command
        .args(args)
        .output()
        .expect("test fixture invariant")
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

pub fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

pub fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("command JSON output")
}

pub fn fingerprint(output: &Output) -> String {
    json(output)["fingerprint"]
        .as_str()
        .expect("fingerprint field")
        .to_owned()
}

pub fn path_text(path: &Path) -> &str {
    path.to_str().expect("test fixture invariant")
}

/// A shipped fixture catalog, used as the content of the author's catalog.
pub fn catalog_fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/signed-catalog")
        .join(name)
}

pub fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).expect("fixture directory");
    for entry in fs::read_dir(source).expect("fixture entries") {
        let entry = entry.expect("fixture entry");
        let target = target.join(entry.file_name());
        if entry.file_type().expect("fixture type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::write(&target, fs::read(entry.path()).expect("fixture bytes"))
                .expect("fixture file");
        }
    }
}

/// Copy a fixture catalog under `root` without a signature, ready for `catalog sign`.
pub fn prepare_catalog(root: &Path, name: &str) -> PathBuf {
    let target = root.join(name);
    copy_tree(&catalog_fixture("v1"), &target);
    for stale in ["catalog.sig", "catalog.pub"] {
        let _ = fs::remove_file(target.join(stale));
    }
    target
}

/// Every regular file under `root`, with its relative path and bytes.
pub fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
    fn collect(root: &Path, directory: &Path, output: &mut Vec<(String, Vec<u8>)>) {
        for entry in fs::read_dir(directory).expect("snapshot entries") {
            let entry = entry.expect("snapshot entry");
            let path = entry.path();
            if entry.file_type().expect("snapshot type").is_dir() {
                collect(root, &path, output);
            } else {
                let name = path
                    .strip_prefix(root)
                    .expect("snapshot path")
                    .to_string_lossy()
                    .replace('\\', "/");
                output.push((name, fs::read(&path).expect("snapshot bytes")));
            }
        }
    }
    let mut output = Vec::new();
    collect(root, root, &mut output);
    output.sort();
    output
}

pub fn git(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {arguments:?}: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git output")
        .trim()
        .to_owned()
}

pub fn init_repo(root: &Path) -> PathBuf {
    let repo = root.join("signed-repo");
    fs::create_dir_all(&repo).expect("repository directory");
    git(&repo, &["init", "--initial-branch=main"]);
    repo
}

pub fn commit_all(repo: &Path, message: &str) -> String {
    git(repo, &["add", "-A"]);
    git(
        repo,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-m",
            message,
        ],
    );
    git(repo, &["rev-parse", "HEAD"])
}
