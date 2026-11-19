use super::*;
use crate::git::{self, GitEnvironment};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

fn environment(hooks: &Path) -> GitEnvironment<'_> {
    GitEnvironment {
        directory: hooks,
        hooks,
        allow_file: true,
    }
}

/// Write a stand-in Git program with one body per platform.
fn write_program(root: &Path, name: &str, windows: &str, unix: &str) -> PathBuf {
    let directory = root.join(name);
    fs::create_dir_all(&directory).expect("program directory");
    if cfg!(windows) {
        let program = directory.join("git.bat");
        fs::write(&program, windows).expect("program");
        program
    } else {
        let program = directory.join("git");
        fs::write(&program, unix).expect("program");
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
    }
}

fn timeout_class(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<FetchError>(),
            Some(FetchError::Timeout)
        )
    })
}

#[test]
fn a_progressing_fetch_outlives_the_inactivity_limit() {
    let root = tempfile::tempdir().expect("temp directory");
    let hooks = git::empty_hooks().expect("empty hooks");
    let program = write_program(
        root.path(),
        "progressing",
        "@echo off\r\nfor /l %%i in (1,1,5) do (\r\n  echo progress %%i 1>&2\r\n  ping -n 2 127.0.0.1 >nul\r\n)\r\nexit /b 0\r\n",
        "#!/bin/sh\nfor i in 1 2 3 4 5; do echo progress $i >&2; sleep 1; done\nexit 0\n",
    );
    let started = Instant::now();
    fetch_with(
        &environment(hooks.path()),
        "file:///progressing",
        &"0".repeat(40),
        Duration::from_millis(2500),
        Some(program.as_os_str()),
    )
    .expect("a fetch that keeps making progress must complete");
    assert!(
        started.elapsed() >= Duration::from_secs(4),
        "the stub ran about five seconds but returned after {:?}",
        started.elapsed()
    );
}

#[test]
fn a_stalled_fetch_fails_as_a_timeout_within_the_limit() {
    let root = tempfile::tempdir().expect("temp directory");
    let hooks = git::empty_hooks().expect("empty hooks");
    let program = write_program(
        root.path(),
        "stalled",
        "@echo off\r\nping -n 60 127.0.0.1 >nul\r\nexit /b 1\r\n",
        "#!/bin/sh\nsleep 60\nexit 1\n",
    );
    let started = Instant::now();
    let error = fetch_with(
        &environment(hooks.path()),
        "file:///stalled",
        &"0".repeat(40),
        Duration::from_millis(800),
        Some(program.as_os_str()),
    )
    .expect_err("a stalled fetch must fail");
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(800),
        "the fetch returned before the inactivity limit: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "the stalled fetch blocked for {elapsed:?}"
    );
    assert!(timeout_class(&error), "{error:#}");
}

#[test]
fn a_stalled_fetch_ends_the_grandchild_before_it_returns() {
    let root = tempfile::tempdir().expect("temp directory");
    let hooks = git::empty_hooks().expect("empty hooks");
    let pidfile = root.path().join("grandchild.pid");
    let path = pidfile.to_string_lossy().replace('\\', "/");
    let windows = format!(
        "@echo off\r\nstart \"\" /b powershell -NoProfile -Command \"$PID | Set-Content -Encoding ascii '{path}'; Start-Sleep -Seconds 60\"\r\n:wait\r\nif not exist \"{path}\" (\r\n  ping -n 1 -w 200 127.0.0.1 >nul\r\n  goto wait\r\n)\r\nping -n 60 127.0.0.1 >nul\r\nexit /b 1\r\n"
    );
    let unix = format!("#!/bin/sh\nsh -c 'echo $$ > \"{path}\"; sleep 60' &\nwait\n");
    let program = write_program(root.path(), "tree", &windows, &unix);
    // Watch the pid file while the fetch runs, so the test knows the pid named
    // a live process and not a stale number.
    let watched = pidfile.clone();
    let watcher = std::thread::spawn(move || watch_pid(&watched, Duration::from_secs(10)));

    let started = Instant::now();
    let error = fetch_with(
        &environment(hooks.path()),
        "file:///tree",
        &"0".repeat(40),
        Duration::from_millis(3000),
        Some(program.as_os_str()),
    )
    .expect_err("a stalled fetch must fail");
    let elapsed = started.elapsed();
    assert!(timeout_class(&error), "{error:#}");
    // The grandchild holds the fetch stderr pipe for a minute, so this call
    // returns only when the tree ended and the reader was joined.
    assert!(
        elapsed < Duration::from_secs(15),
        "the stalled fetch blocked for {elapsed:?}"
    );

    let (pid, alive) = watcher.join().expect("pid watcher");
    assert!(alive, "the pid file named {pid}, which never ran");
    assert!(
        wait_until_gone(&pid, Duration::from_secs(10)),
        "the grandchild process {pid} outlived the timed-out fetch"
    );
}

#[test]
fn a_low_speed_abort_keeps_the_timeout_class() {
    let root = tempfile::tempdir().expect("temp directory");
    let hooks = git::empty_hooks().expect("empty hooks");
    let message = "error: RPC failed; curl 28 Operation too slow. Less than 1 bytes/sec transferred the last 60 seconds";
    let windows = format!("@echo off\r\necho {message} 1>&2\r\nexit /b 128\r\n");
    let unix = format!("#!/bin/sh\necho \"{message}\" >&2\nexit 128\n");
    let program = write_program(root.path(), "low-speed", &windows, &unix);
    let error = fetch_with(
        &environment(hooks.path()),
        "file:///low-speed",
        &"0".repeat(40),
        Duration::from_secs(30),
        Some(program.as_os_str()),
    )
    .expect_err("a low-speed abort must fail");
    assert!(timeout_class(&error), "{error:#}");
    assert!(format!("{error:#}").contains("low-speed"), "{error:#}");
}

/// Wait for the stub to write its grandchild pid and report whether that pid
/// named a live process when it first appeared.
fn watch_pid(path: &Path, limit: Duration) -> (String, bool) {
    let deadline = Instant::now() + limit;
    loop {
        if let Ok(text) = fs::read_to_string(path) {
            let pid = text.trim().to_owned();
            if !pid.is_empty() {
                let alive = process_alive(&pid);
                return (pid, alive);
            }
        }
        assert!(
            Instant::now() < deadline,
            "the stub did not write its grandchild pid"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_until_gone(pid: &str, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        if !process_alive(pid) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(windows)]
fn process_alive(pid: &str) -> bool {
    let output = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output();
    match output {
        Ok(output) => String::from_utf8_lossy(&output.stdout).contains(pid),
        Err(_) => false,
    }
}

#[cfg(unix)]
fn process_alive(pid: &str) -> bool {
    matches!(
        Command::new("kill").args(["-0", pid]).status(),
        Ok(status) if status.success()
    )
}
