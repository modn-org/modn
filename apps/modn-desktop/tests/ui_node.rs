//! Runs the desktop UI behaviour tests with the Node test runner.
//!
//! The UI ships as plain ES modules, so the behaviour tests under `tests/js`
//! import the real modules and stub `window`, `document` and timers. Node is a
//! developer tool, not a build requirement. When `node` is not on PATH a local
//! run prints a skip message and passes. When `CI` holds a non-empty value the
//! same situation fails the test, so continuous integration cannot hide UI
//! behaviour failures.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

/// What asking `node --version` produced.
#[derive(Debug, PartialEq, Eq)]
enum NodeProbe {
    /// Node ran and reported this version, for example `v22.1.0`.
    Version(String),
    /// Starting `node` failed because no executable is on PATH.
    NotFound,
}

/// What the test does with a [`NodeProbe`] result.
#[derive(Debug, PartialEq, Eq)]
enum Decision {
    /// Run `node --test` with the reported version.
    Run { version: String },
    /// Report that the tests were skipped and pass.
    Skip { message: String },
    /// Fail the test because a CI run must exercise the UI behaviour tests.
    Fail { message: String },
}

/// Decides how to treat a Node probe result.
///
/// CI must not skip the UI behaviour tests, so a missing `node` fails there.
/// A local run keeps the skip so that Node stays optional for developers.
fn node_policy(ci: bool, probe: NodeProbe) -> Decision {
    match probe {
        NodeProbe::Version(version) => Decision::Run { version },
        NodeProbe::NotFound if ci => Decision::Fail {
            message: "CI is set and node is not on PATH: the UI behaviour tests cannot run. \
                      Install node and add it to PATH."
                .to_owned(),
        },
        NodeProbe::NotFound => Decision::Skip {
            message: "skipping UI behaviour tests: node is not on PATH".to_owned(),
        },
    }
}

/// Reports whether `CI` holds a non-empty value.
fn ci_environment() -> bool {
    env::var_os("CI").is_some_and(|value| !value.is_empty())
}

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
    let probe = match Command::new("node").arg("--version").output() {
        Ok(output) => NodeProbe::Version(String::from_utf8_lossy(&output.stdout).trim().to_owned()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => NodeProbe::NotFound,
        Err(error) => panic!("cannot run node for the UI behaviour tests: {error}"),
    };
    let version = match node_policy(ci_environment(), probe) {
        Decision::Run { version } => version,
        Decision::Skip { message } => {
            eprintln!("{message}");
            return;
        }
        Decision::Fail { message } => panic!("{message}"),
    };
    println!("node {version} runs the UI behaviour tests");
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

#[test]
fn ci_with_missing_node_fails() {
    match node_policy(true, NodeProbe::NotFound) {
        Decision::Fail { message } => {
            assert!(
                message.contains("CI"),
                "the message must name CI: {message}"
            );
            assert!(
                message.contains("node"),
                "the message must name node: {message}"
            );
            assert!(
                message.contains("PATH"),
                "the message must name PATH: {message}"
            );
        }
        other => panic!("CI with a missing node must fail, got {other:?}"),
    }
}

#[test]
fn local_run_with_missing_node_skips() {
    match node_policy(false, NodeProbe::NotFound) {
        Decision::Skip { message } => {
            assert!(
                message.contains("node"),
                "the message must name node: {message}"
            );
        }
        other => panic!("a local run with a missing node must skip, got {other:?}"),
    }
}

#[test]
fn node_version_runs_the_tests() {
    for ci in [false, true] {
        let decision = node_policy(ci, NodeProbe::Version("v22.1.0".to_owned()));
        assert_eq!(
            decision,
            Decision::Run {
                version: "v22.1.0".to_owned()
            },
            "a known node version must run the tests (CI {ci})"
        );
    }
}

#[test]
fn the_ui_calls_the_rust_clipboard_command() {
    let api = fs::read_to_string(Path::new(ROOT).join("ui/js/api.js")).expect("API module");
    assert!(
        api.contains("invoke('copy_diagnostics', {})"),
        "the API must invoke the copy_diagnostics command"
    );
    let mut pending = vec![Path::new(ROOT).join("ui/js")];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).expect("UI script directory") {
            let path = entry.expect("UI script entry").path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            if path.extension().and_then(|extension| extension.to_str()) != Some("js") {
                continue;
            }
            let text = fs::read_to_string(&path).expect("UI script");
            assert!(
                !text.contains("navigator.clipboard"),
                "{path:?} must use the Rust clipboard command"
            );
        }
    }
}
