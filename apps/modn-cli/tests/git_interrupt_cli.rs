//! A real SIGINT stops the CLI and the whole stub Git process tree.
//!
//! The stub Git program re-runs this test binary as a three-process tree that
//! records every pid and blocks forever. The test sends `SIGINT` to the CLI
//! process only, then checks the cancelled outcome and that every recorded
//! stub process is gone.
//!
//! The test runs on Unix only. It uses `kill` from the system, because the CLI
//! crate forbids `unsafe` and has no signal binding of its own.
#![cfg(unix)]

#[path = "support/catalog.rs"]
mod support;

#[path = "support/git_fixture.rs"]
mod git_fixture;

#[path = "support/git_source_helpers.rs"]
mod helpers;

#[path = "support/main_bucket.rs"]
mod main_bucket;

use git_fixture::{file_url, repository};
use helpers::add_bucket;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use support::Fixture;

/// The environment variable that names the role of a re-run stub process.
const STUB_ROLE: &str = "MODN_TEST_CLI_GIT_ROLE";
/// The environment variable that names the pid file shared by a stub tree.
const STUB_PIDS: &str = "MODN_TEST_CLI_GIT_PIDS";
/// The test filter that turns a re-run of this binary into a stub.
const STUB_TEST: &str = "stub_entry";

/// The bounded wait for the stub tree, the CLI exit and the tree teardown.
const WAIT: Duration = Duration::from_secs(30);

const POLL: Duration = Duration::from_millis(20);

/// The stub entry point. A normal test run has no role and returns at once; a
/// re-run with [`STUB_ROLE`] becomes the requested tree member.
#[test]
fn stub_entry() {
    let Ok(role) = std::env::var(STUB_ROLE) else {
        return;
    };
    let pids = PathBuf::from(std::env::var_os(STUB_PIDS).expect("stub pid file"));
    act_as_stub(&role, &pids);
}

/// Run the role of one stub process. Only blocking or `exec` follows.
fn act_as_stub(role: &str, pids: &Path) -> ! {
    match role {
        "parent" => {
            record_pid(pids);
            spawn_stub("child", pids);
            block_forever();
        }
        "child" => {
            record_pid(pids);
            spawn_stub("grandchild", pids);
            block_forever();
        }
        "grandchild" => {
            record_pid(pids);
            block_forever();
        }
        other => {
            eprintln!("unknown stub role {other}");
            std::process::exit(2);
        }
    }
}

fn block_forever() -> ! {
    loop {
        std::thread::sleep(Duration::from_secs(30));
    }
}

fn record_pid(pids: &Path) {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(pids)
        .expect("stub pid file");
    writeln!(file, "{}", std::process::id()).expect("stub pid write");
}

#[expect(
    clippy::zombie_processes,
    reason = "the stub child blocks until the runner stops the whole tree; this process never reaps it"
)]
fn spawn_stub(role: &str, pids: &Path) {
    let program = std::env::current_exe().expect("the test binary path");
    Command::new(program)
        .arg(STUB_TEST)
        .env(STUB_ROLE, role)
        .env(STUB_PIDS, pids)
        .spawn()
        .expect("stub process");
}

/// Write the shim that the CLI starts as Git. The shim delegates every command
/// except `fetch` to the real Git, so the private repository still initialises.
fn stub_program(root: &Path, pids: &Path) -> PathBuf {
    let directory = root.join("stub-git");
    fs::create_dir_all(&directory).expect("stub program directory");
    let program = directory.join("git");
    let exe = std::env::current_exe().expect("the test binary path");
    let script = format!(
        "#!/bin/sh\ncase \"$*\" in\n  *fetch*)\n    {STUB_ROLE}=parent {STUB_PIDS}=\"{}\" exec \"{}\" {STUB_TEST}\n    ;;\n  *)\n    exec git \"$@\"\n    ;;\nesac\n",
        pids.display(),
        exe.display()
    );
    fs::write(&program, script).expect("stub program");
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(&program)
        .expect("program metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&program, permissions).expect("program permissions");
    program
}

/// The CLI child and its stub pids, killed on drop so a failed assertion leaves
/// no process behind.
struct Running {
    child: Option<Child>,
    pids: PathBuf,
}

impl Running {
    fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("the CLI child is alive")
    }

    fn wait_bounded(&mut self) -> std::process::ExitStatus {
        let deadline = Instant::now() + WAIT;
        loop {
            match self.child_mut().try_wait() {
                Ok(Some(status)) => {
                    self.child = None;
                    return status;
                }
                Ok(None) => {}
                Err(error) => panic!("cannot poll the CLI child: {error}"),
            }
            assert!(
                Instant::now() < deadline,
                "the CLI did not exit within {WAIT:?} of the SIGINT"
            );
            std::thread::sleep(POLL);
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        for pid in read_pids(&self.pids) {
            let _ = Command::new("kill")
                .args(["-KILL", &pid.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

fn read_pids(pids: &Path) -> Vec<u32> {
    fs::read_to_string(pids)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect()
}

fn wait_for_tree(pids: &Path) -> bool {
    let deadline = Instant::now() + WAIT;
    loop {
        if read_pids(pids).len() >= 3 {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(POLL);
    }
}

fn process_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn wait_until_tree_dead(pids: &Path) {
    let recorded = read_pids(pids);
    assert!(
        recorded.len() >= 3,
        "the stub tree reported {} of its 3 processes: {recorded:?}",
        recorded.len()
    );
    let deadline = Instant::now() + WAIT;
    loop {
        let alive: Vec<u32> = recorded
            .iter()
            .copied()
            .filter(|pid| process_alive(*pid))
            .collect();
        if alive.is_empty() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "stub processes {alive:?} are still alive {WAIT:?} after the CLI exited"
        );
        std::thread::sleep(POLL);
    }
}

#[test]
fn a_real_sigint_stops_the_cli_and_the_stub_git_tree() {
    let fixture = Fixture::new();
    let repository = repository(fixture.temp.path());
    add_bucket(&fixture, fixture.temp.path(), &repository.commit);
    fixture.register();
    let pids = fixture.temp.path().join("stub-pids");
    let program = stub_program(fixture.temp.path(), &pids);
    let stderr_path = fixture.temp.path().join("cli-stderr.txt");
    let stderr = fs::File::create(&stderr_path).expect("stderr capture");

    let mut seeded = main_bucket::command(env!("CARGO_BIN_EXE_modn"));
    seeded
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .env("MODN_TEST_GIT_REMOTE_BASE", file_url(&repository.base))
        .env("MODN_TEST_GIT_FETCH_PROGRAM", &program)
        .args([
            "--json",
            "install",
            "test-game/git-package",
            "--path",
            fixture.game.to_str().expect("game path"),
            "--yes",
        ])
        .stdout(Stdio::null())
        .stderr(stderr);
    let mut running = Running {
        child: Some(seeded.spawn().expect("run the CLI")),
        pids: pids.clone(),
    };

    assert!(
        wait_for_tree(&pids),
        "the stub Git tree did not reach three processes within {WAIT:?}"
    );
    let signal = Command::new("kill")
        .args(["-INT", &running.child_mut().id().to_string()])
        .status()
        .expect("send SIGINT");
    assert!(signal.success(), "the SIGINT command failed: {signal}");

    let status = running.wait_bounded();
    assert_eq!(
        status.code(),
        Some(1),
        "the CLI must report a cancelled command, not the 130 backstop: {status}"
    );
    let stderr = fs::read_to_string(&stderr_path).expect("captured stderr");
    assert!(
        stderr.contains("\"error\":\"CANCELLED\""),
        "the CLI did not report the cancelled outcome: {stderr}"
    );
    wait_until_tree_dead(&pids);
    assert!(
        !fixture.game.join("addons").exists(),
        "a cancelled install must not change game files"
    );
}
