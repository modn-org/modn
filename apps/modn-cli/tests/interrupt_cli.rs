#![cfg(unix)]
//! A Unix signal ends the live Git fetch tree and keeps the signal exit status.

#[path = "support/catalog.rs"]
mod support;

#[path = "support/git_fixture.rs"]
mod git_fixture;

#[path = "support/git_source.rs"]
mod git_source;

#[path = "support/main_bucket.rs"]
mod main_bucket;

use git_fixture::repository;
use git_source::add_stalled_then_serving_bucket;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};
use support::Fixture;

#[test]
fn sigint_ends_the_fetch_tree_and_exits_with_130() {
    run_interrupted_install("-INT", 130);
}

#[test]
fn sigterm_ends_the_fetch_tree_and_exits_with_143() {
    run_interrupted_install("-TERM", 143);
}

/// Install through a stalled stub Git, send one signal, and check the result.
fn run_interrupted_install(signal: &str, expected: i32) {
    let fixture = Fixture::new();
    let repository = repository(fixture.temp.path());
    add_stalled_then_serving_bucket(&fixture, fixture.temp.path(), &repository.commit);
    fixture.register();
    let heartbeat = fixture.temp.path().join("interrupt.beats");
    let program = interrupt_git_program(fixture.temp.path(), &heartbeat);
    let capture = fixture.temp.path().join("interrupt-stderr.txt");
    let stderr = fs::File::create(&capture).expect("stderr capture");
    let mut command = main_bucket::command(env!("CARGO_BIN_EXE_modn"));
    command
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .env(
            "MODN_TEST_GIT_REMOTE_BASE",
            git_fixture::file_url(&repository.base),
        )
        .env("MODN_TEST_GIT_FETCH_PROGRAM", &program)
        .env("MODN_TEST_GIT_FETCH_IDLE_MS", "60000")
        .args([
            "install",
            "test-game/git-package",
            "--path",
            fixture.game.to_str().expect("game path"),
            "--yes",
        ])
        .stdout(Stdio::null())
        .stderr(stderr);
    let mut child = command.spawn().expect("run CLI");
    let pid = child.id();
    // The stub writes its heartbeat only after its process group exists, so a
    // signal sent now always finds the registered tree.
    wait_for_heartbeat(&heartbeat);
    let sent = Command::new("kill")
        .arg(signal)
        .arg(pid.to_string())
        .status()
        .expect("send the signal");
    assert!(sent.success(), "kill {signal} {pid} failed");
    let status = wait_for_exit(&mut child);
    assert_eq!(
        status.code(),
        Some(expected),
        "status {status:?}\n{}",
        fs::read_to_string(&capture).unwrap_or_default()
    );
    // The grandchild wrote a heartbeat every 50 ms. Give an in-flight write
    // time to finish, then require that no later write appears.
    std::thread::sleep(Duration::from_millis(300));
    let settled = fs::metadata(&heartbeat).map(|file| file.len()).unwrap_or(0);
    std::thread::sleep(Duration::from_millis(500));
    let later = fs::metadata(&heartbeat).map(|file| file.len()).unwrap_or(0);
    assert_eq!(
        settled, later,
        "the Git fetch tree kept writing heartbeats after {signal}"
    );
}

/// A stub Git that stalls on `fixture/stalled` with a grandchild heartbeat.
fn interrupt_git_program(root: &Path, heartbeat: &Path) -> PathBuf {
    let beats = heartbeat.to_string_lossy().replace('\\', "/");
    let unix = format!(
        "sh -c 'while :; do echo tick >> \"{beats}\"; sleep 0.05; done' &\nsleep 60\nexit 1"
    );
    git_source::stub_git_program(root, "interrupt-git", "fixture/stalled", "", &unix)
}

/// Wait until the stub's grandchild has written at least one heartbeat.
fn wait_for_heartbeat(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while fs::metadata(path).map(|file| file.len()).unwrap_or(0) == 0 {
        assert!(
            Instant::now() < deadline,
            "the stub Git never wrote a heartbeat"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Wait for the CLI to exit, so a broken handler fails instead of hanging.
fn wait_for_exit(child: &mut Child) -> ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(status) = child.try_wait().expect("wait for CLI") {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "the CLI did not exit after the signal"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
