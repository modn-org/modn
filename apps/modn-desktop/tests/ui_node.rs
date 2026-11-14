//! Runs the desktop UI behaviour tests with the Node test runner.
//!
//! The UI ships as plain ES modules, so the behaviour tests under `tests/js`
//! import the real modules and stub `window`, `document` and timers. Node is a
//! developer tool, not a build requirement: a machine without `node` on PATH
//! prints a skip message and passes.

use std::path::Path;
use std::process::Command;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

#[test]
fn ui_behaviour_tests_pass() {
    let directory = Path::new(ROOT).join("tests").join("js");
    assert!(
        directory.is_dir(),
        "the UI behaviour tests must live in {directory:?}"
    );
    let output = match Command::new("node").arg("--test").arg(&directory).output() {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("skipping UI behaviour tests: node is not on PATH");
            return;
        }
        Err(error) => panic!("cannot run node for the UI behaviour tests: {error}"),
    };
    assert!(
        output.status.success(),
        "node --test failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
