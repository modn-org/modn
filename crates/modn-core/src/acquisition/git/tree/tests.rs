//! Disposable stub Git trees.
//!
//! Each stub is this test binary, re-run with [`STUB_TEST`] as the test filter
//! and [`STUB_ROLE`] as its role. A parent starts a child, and the child starts
//! a grandchild, so the tree has the shape of Git plus a remote helper. Every
//! member blocks until the runner stops it. The pids are written to one file,
//! so a test can check that every process is dead after the runner returns.
//!
//! The test binary is used instead of a batch script because a batch stub
//! needs `cmd.exe` and a `ping` grandchild to hold inherited pipes. The binary
//! keeps the tree and its exit codes identical on Windows and Unix.
#![expect(
    unsafe_code,
    reason = "the Windows pid check calls OpenProcess, GetExitCodeProcess and CloseHandle"
)]

use super::ForcedUnconfirmed;
use crate::acquisition::git::run::{self, GitOutput, RunError};
use crate::git;
use crate::progress::CancellationToken;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// The environment variable that names the role of a re-run stub process.
const STUB_ROLE: &str = "MODN_TEST_TREE_STUB";
/// The environment variable that names the pid file shared by a stub tree.
const STUB_PIDS: &str = "MODN_TEST_TREE_PIDS";
/// The test filter that turns a re-run of this binary into a stub.
const STUB_TEST: &str = "tree::tests::stub_entry";

/// A deadline that is long enough for the stub tree to form on a slow CI
/// machine, and short enough that the test is quick.
const STUB_DEADLINE: Duration = Duration::from_secs(10);

/// A deadline that only guards against a broken test that never finishes.
const NO_DEADLINE: Duration = Duration::from_secs(120);

/// The bounded wait for a stub tree to report its pids.
const PID_WAIT: Duration = Duration::from_secs(30);

/// The test program seam for this thread, removed when the test ends.
struct Program(Option<std::ffi::OsString>);

impl Program {
    fn install(path: &Path) -> Self {
        let previous =
            crate::git::TEST_PROGRAM.with(|slot| slot.replace(Some(path.as_os_str().to_owned())));
        Self(previous)
    }
}

impl Drop for Program {
    fn drop(&mut self) {
        crate::git::TEST_PROGRAM.with(|slot| slot.replace(self.0.take()));
    }
}

/// The stub entry point. A normal test run has no role and returns at once;
/// a re-run with [`STUB_ROLE`] becomes the requested tree member.
#[test]
fn stub_entry() {
    let Ok(role) = std::env::var(STUB_ROLE) else {
        return;
    };
    let pids = PathBuf::from(std::env::var_os(STUB_PIDS).expect("stub pid file"));
    act_as_stub(&role, &pids);
}

/// Run the role of one stub process. Only `exit` returns out of this.
fn act_as_stub(role: &str, pids: &Path) -> ! {
    match role {
        "parent" => {
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
        "leaving-grandchild" => {
            // The grandchild inherits the runner's stderr pipe, so the
            // reader joins only after the tree stop reaches it.
            spawn_stub("grandchild", pids);
            wait_for_pid_file(pids);
            std::process::exit(1);
        }
        "exiting" => std::process::exit(1),
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

fn spawn_stub(role: &str, pids: &Path) {
    let program = std::env::current_exe().expect("the test binary path");
    Command::new(program)
        .arg(STUB_TEST)
        .env(STUB_ROLE, role)
        .env(STUB_PIDS, pids)
        .spawn()
        .expect("stub process");
}

fn wait_for_pid_file(pids: &Path) {
    let deadline = Instant::now() + PID_WAIT;
    while !has_pid_line(pids) {
        if Instant::now() >= deadline {
            eprintln!("the stub grandchild did not write its pid within {PID_WAIT:?}");
            std::process::exit(3);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn has_pid_line(pids: &Path) -> bool {
    fs::read_to_string(pids).is_ok_and(|contents| {
        contents
            .lines()
            .any(|line| line.trim().parse::<u32>().is_ok())
    })
}

/// Write the shim that the runner starts as Git. It re-runs this test binary
/// with the requested stub role.
fn stub_program(root: &Path, role: &str, pids: &Path) -> PathBuf {
    let directory = root.join("stub-git");
    fs::create_dir_all(&directory).expect("stub program directory");
    let program = if cfg!(windows) {
        let program = directory.join("git.bat");
        let exe = std::env::current_exe().expect("the test binary path");
        let script = format!(
            "@echo off\r\nset \"{STUB_ROLE}={role}\"\r\nset \"{STUB_PIDS}={}\"\r\n\"{}\" {STUB_TEST}\r\n",
            pids.display(),
            exe.display()
        );
        fs::write(&program, script).expect("stub program");
        program
    } else {
        let program = directory.join("git");
        let exe = std::env::current_exe().expect("the test binary path");
        let script = format!(
            "#!/bin/sh\n{STUB_ROLE}={role} {STUB_PIDS}=\"{}\" exec \"{}\" {STUB_TEST}\n",
            pids.display(),
            exe.display()
        );
        fs::write(&program, script).expect("stub program");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(&program)
                .expect("program metadata")
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&program, permissions).expect("program permissions");
        }
        program
    };
    program
}

/// Run the shared Git command against the installed stub program.
fn run_stub(token: &CancellationToken, deadline: Duration) -> Result<GitOutput, RunError> {
    let directory = tempfile::tempdir().expect("stub working directory");
    let hooks = git::empty_hooks().expect("empty hooks");
    let mut command = git::command(&git::GitEnvironment {
        directory: directory.path(),
        hooks: hooks.path(),
        allow_file: true,
    });
    command.args([
        "fetch",
        "--",
        "https://example.invalid/stub/project.git",
        "0123456789abcdef0123456789abcdef01234567",
    ]);
    run::run(command, token, deadline)
}

/// Read the pids that the stub tree reported, and fail clearly when it did not
/// report at least `minimum` members.
fn stub_pids(pids: &Path, minimum: usize) -> Vec<u32> {
    let contents = fs::read_to_string(pids).unwrap_or_default();
    let pids: Vec<u32> = contents
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect();
    assert!(
        pids.len() >= minimum,
        "the stub tree reported {} of its {minimum} expected processes within {PID_WAIT:?}: {contents:?}",
        pids.len()
    );
    pids
}

/// Poll until the stub tree reported both descendants, or the bounded wait
/// elapses.
fn wait_for_pids(pids: &Path) -> bool {
    let deadline = Instant::now() + PID_WAIT;
    loop {
        let reported = fs::read_to_string(pids).map(|contents| {
            contents
                .lines()
                .filter(|line| !line.trim().is_empty())
                .count()
        });
        if reported.is_ok_and(|count| count >= 2) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Check that every pid of the stub tree is gone.
fn assert_tree_dead(pids: &Path, minimum: usize) {
    for pid in stub_pids(pids, minimum) {
        assert!(
            !process_alive(pid),
            "stub process {pid} is still alive after the runner returned"
        );
    }
}

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    // SAFETY: The call takes plain values and returns a handle that this
    // function closes once.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return false;
    }
    let mut code = 0_u32;
    // SAFETY: `process` is a live process handle and `code` is writable.
    let queried = unsafe { GetExitCodeProcess(process, &mut code) };
    // SAFETY: The handle is live and is closed once here.
    unsafe {
        CloseHandle(process);
    }
    queried != 0 && code == STILL_ACTIVE as u32
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    use rustix::io::Errno;
    use rustix::process::{test_kill_process, Pid};

    match Pid::from_raw(pid as i32) {
        Some(pid) => match test_kill_process(pid) {
            Ok(()) => true,
            Err(Errno::ESRCH) => false,
            Err(_) => true,
        },
        None => false,
    }
}

#[test]
fn a_hard_deadline_stops_the_whole_stub_tree() {
    let root = tempfile::tempdir().expect("temp");
    let pids = root.path().join("pids");
    let program = stub_program(root.path(), "parent", &pids);
    let _program = Program::install(&program);

    let error = run_stub(&CancellationToken::new(), STUB_DEADLINE)
        .err()
        .expect("a stub tree that never finishes must hit the hard deadline");
    assert!(matches!(error, RunError::Timeout), "{error:?}");
    assert_tree_dead(&pids, 2);
}

#[test]
fn a_cancel_stops_the_whole_stub_tree() {
    let root = tempfile::tempdir().expect("temp");
    let pids = root.path().join("pids");
    let program = stub_program(root.path(), "parent", &pids);
    let _program = Program::install(&program);
    let token = CancellationToken::new();
    let watcher = std::thread::spawn({
        let pids = pids.clone();
        let token = token.clone();
        move || {
            let formed = wait_for_pids(&pids);
            token.cancel();
            formed
        }
    });

    let error = run_stub(&token, NO_DEADLINE)
        .err()
        .expect("a cancelled fetch must fail");
    let formed = watcher.join().expect("watcher thread");
    assert!(formed, "the stub tree did not form within {PID_WAIT:?}");
    assert!(matches!(error, RunError::Cancelled), "{error:?}");
    assert_tree_dead(&pids, 2);
}

#[test]
fn a_failed_exit_still_stops_the_whole_stub_tree() {
    let root = tempfile::tempdir().expect("temp");
    let pids = root.path().join("pids");
    let program = stub_program(root.path(), "leaving-grandchild", &pids);
    let _program = Program::install(&program);

    let output = run_stub(&CancellationToken::new(), NO_DEADLINE)
        .expect("a failed Git exit is an ordinary output");
    assert!(
        !output.success(),
        "the stub parent must fail while its grandchild still runs"
    );
    assert_tree_dead(&pids, 1);
}

#[test]
fn an_unconfirmed_stop_returns_the_stable_tree_error() {
    let root = tempfile::tempdir().expect("temp");
    let pids = root.path().join("pids");
    let program = stub_program(root.path(), "exiting", &pids);
    let _program = Program::install(&program);
    let _forced = ForcedUnconfirmed::install();

    let error = run_stub(&CancellationToken::new(), NO_DEADLINE)
        .err()
        .expect("a forced confirmation failure must fail the fetch");
    assert!(
        matches!(error, RunError::Tree(super::TreeError::Unconfirmed)),
        "{error:?}"
    );
}
