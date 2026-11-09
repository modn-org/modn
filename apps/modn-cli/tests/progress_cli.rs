#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "support/catalog.rs"]
mod support;
#[path = "support/tree.rs"]
#[allow(dead_code)]
mod tree;

use serde_json::Value;
use std::io::Write;
use std::path::Path;
use std::process::{Output, Stdio};
use support::{error, Fixture};

fn phases(stderr: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(stderr)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|value| value["phase"].as_str().map(str::to_owned))
        .collect()
}

fn cancelled_install(fixture: &Fixture, archive: &Path) -> Output {
    let mut command = main_bucket::command(env!("CARGO_BIN_EXE_modn"));
    command
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .args([
            "--json",
            "--progress",
            "json",
            "install",
            "example",
            "--file",
        ])
        .arg(archive)
        .arg("--yes")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn modn");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"cancel\n")
        .expect("cancel line");
    drop(child.stdin.take());
    child.wait_with_output().expect("modn output")
}

#[test]
fn progress_json_reports_every_install_phase_on_stderr() {
    let fixture = Fixture::new();
    let (archive, digest) = fixture.archive();
    fixture.bucket("deps", "test-game", "example", &digest, &[]);
    fixture.register();
    let before = tree::snapshot(&fixture.game);

    let output = fixture.run_paths(
        &[
            "--json",
            "--progress",
            "json",
            "install",
            "example",
            "--yes",
            "--file",
        ],
        &[archive.as_path()],
    );
    assert!(output.status.success(), "{}", error(&output));
    let report: Value = serde_json::from_slice(&output.stdout).expect("stdout JSON");
    assert!(report["installed"].is_object(), "{report}");
    assert_eq!(
        phases(&output.stderr),
        ["resolve", "extract", "stage", "commit", "result"]
    );
    assert!(fixture.game.join("addons/example.plugin").is_file());
    assert_ne!(tree::snapshot(&fixture.game), before);
}

#[test]
fn a_cancel_line_stops_the_install_before_the_commit() {
    let fixture = Fixture::new();
    let (archive, digest) = fixture.archive();
    fixture.bucket("deps", "test-game", "example", &digest, &[]);
    fixture.register();
    let before = tree::snapshot(&fixture.game);

    let output = cancelled_install(&fixture, &archive);
    assert!(!output.status.success());
    assert_eq!(tree::snapshot(&fixture.game), before);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(r#""error":"CANCELLED""#), "{stderr}");
    let phases = phases(&output.stderr);
    assert_eq!(phases.last().map(String::as_str), Some("result"));
    let cancelled: Vec<Value> = stderr
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|value| value["outcome"] == "cancelled")
        .collect();
    assert_eq!(cancelled.len(), 1, "{stderr}");
}

#[test]
fn touched_commands_never_report_a_generic_error_code() {
    let fixture = Fixture::new();
    fixture.register();
    let install = fixture.run(&["--json", "install", "missing-mod"]);
    assert_eq!(error_code(&install), "MOD_NOT_FOUND");

    let update = fixture.run(&["--json", "update", "missing-mod"]);
    assert_eq!(error_code(&update), "MOD_NOT_FOUND");

    let uninstall = fixture.run(&["--json", "uninstall", "anything"]);
    assert_eq!(error_code(&uninstall), "APPROVAL_REQUIRED");

    for output in [&install, &update, &uninstall] {
        assert_ne!(error_code(output), "ERROR");
    }
}

fn error_code(output: &Output) -> String {
    let value: Value = serde_json::from_slice(&output.stderr).expect("error JSON");
    value["error"].as_str().expect("error code").to_owned()
}
