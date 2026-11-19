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
        TreeStop::End,
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
        TreeStop::End,
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
fn an_unstoppable_tree_is_a_hard_error_not_a_timeout() {
    let root = tempfile::tempdir().expect("temp directory");
    let hooks = git::empty_hooks().expect("empty hooks");
    let program = write_program(
        root.path(),
        "unstoppable",
        "@echo off\r\nping -n 60 127.0.0.1 >nul\r\nexit /b 1\r\n",
        "#!/bin/sh\nsleep 60\nexit 1\n",
    );
    let error = fetch_with(
        &environment(hooks.path()),
        "file:///unstoppable",
        &"0".repeat(40),
        Duration::from_millis(800),
        Some(program.as_os_str()),
        TreeStop::Fail,
    )
    .expect_err("a tree that cannot be stopped must fail");
    // A timeout attempt lets the caller try another source. A tree that will
    // not end must not: the error names the stopped tree and is not a timeout.
    assert!(!timeout_class(&error), "{error:#}");
    assert!(is_tree_still_running(&error), "{error:#}");
    assert!(
        format!("{error:#}").contains("process tree could not be stopped"),
        "{error:#}"
    );
}

#[test]
fn a_stalled_fetch_ends_the_grandchild_before_it_returns() {
    let root = tempfile::tempdir().expect("temp directory");
    let hooks = git::empty_hooks().expect("empty hooks");
    let pidfile = root.path().join("grandchild.pid");
    let heartbeat = root.path().join("grandchild.beats");
    let pid = path_for_stub(&pidfile);
    let beats = path_for_stub(&heartbeat);
    // The stub writes a heartbeat to stderr until the grandchild has written
    // its pid. Every line resets the inactivity timer, so a slow Windows runner
    // that needs several seconds to start PowerShell is never mistaken for a
    // stalled transfer. Only after the pid exists does the stub go silent.
    let windows = format!(
        "@echo off\r\nstart \"\" /b powershell -NoProfile -Command \"$PID | Set-Content -Encoding ascii '{pid}'; for ($i = 0; $i -lt 1200; $i++) {{ Add-Content -Encoding ascii -Path '{beats}' -Value 'tick'; Start-Sleep -Milliseconds 50 }}\"\r\n:wait\r\nif exist \"{pid}\" goto silent\r\necho starting 1>&2\r\nping -n 1 -w 200 127.0.0.1 >nul\r\ngoto wait\r\n:silent\r\nping -n 60 127.0.0.1 >nul\r\nexit /b 1\r\n"
    );
    let unix = format!(
        "#!/bin/sh\nsh -c 'echo $$ > \"{pid}\"; while :; do echo tick >> \"{beats}\"; sleep 0.05; done' &\nwhile [ ! -f \"{pid}\" ]; do echo starting >&2; sleep 0.2; done\nsleep 60\nexit 1\n"
    );
    let program = write_program(root.path(), "tree", &windows, &unix);
    // Watch the pid file while the fetch runs, so the test knows the pid named
    // a live process and not a stale number. The deadline only bounds a broken
    // stub; no assertion below measures how fast a process starts.
    let watched = pidfile.clone();
    let watcher = std::thread::spawn(move || watch_pid(&watched, Duration::from_secs(30)));

    let started = Instant::now();
    let error = fetch_with(
        &environment(hooks.path()),
        "file:///tree",
        &"0".repeat(40),
        Duration::from_millis(3000),
        Some(program.as_os_str()),
        TreeStop::End,
    )
    .expect_err("a stalled fetch must fail");
    let elapsed = started.elapsed();
    assert!(timeout_class(&error), "{error:#}");
    // The grandchild holds the fetch stderr pipe for a minute, so this call
    // returns only after the tree ended and its reader was joined.
    assert!(
        elapsed < Duration::from_secs(20),
        "the stalled fetch blocked for {elapsed:?}"
    );

    let (pid, alive) = watcher.join().expect("pid watcher");
    assert!(alive, "the pid file named {pid}, which never ran");
    // The grandchild appends to its heartbeat file every 50 ms. A process that
    // still runs would keep writing after the fetch returned; a terminated one
    // cannot. Read the size right after the return, then again later, so the
    // check needs no polling window and cannot fail on a slow runner.
    let settled = fs::metadata(&heartbeat).map(|file| file.len()).unwrap_or(0);
    std::thread::sleep(Duration::from_millis(500));
    let later = fs::metadata(&heartbeat).map(|file| file.len()).unwrap_or(0);
    assert_eq!(
        settled, later,
        "the grandchild of {pid} still ran after the timed-out fetch returned"
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
        TreeStop::End,
    )
    .expect_err("a low-speed abort must fail");
    assert!(timeout_class(&error), "{error:#}");
    assert!(format!("{error:#}").contains("low-speed"), "{error:#}");
}

/// Render a Windows path for an inline program, with forward slashes.
fn path_for_stub(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
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

#[test]
fn a_fetch_registers_its_tree_until_the_fetch_ends() {
    let root = tempfile::tempdir().expect("temp directory");
    let program = write_program(
        root.path(),
        "registered",
        "@echo off\r\nping -n 60 127.0.0.1 >nul\r\nexit /b 1\r\n",
        "#!/bin/sh\nsleep 60\nexit 1\n",
    );
    let fetch = std::thread::spawn(move || {
        let hooks = git::empty_hooks().expect("empty hooks");
        fetch_with(
            &environment(hooks.path()),
            "file:///registered",
            &"0".repeat(40),
            Duration::from_secs(3),
            Some(program.as_os_str()),
            TreeStop::End,
        )
    });
    // The fetch registers its tree while it runs.
    let deadline = Instant::now() + Duration::from_secs(30);
    while super::registry::active_count() == 0 {
        assert!(
            Instant::now() < deadline,
            "the running fetch did not register a tree"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let error = fetch
        .join()
        .expect("fetch thread")
        .expect_err("a stalled fetch must fail");
    assert!(timeout_class(&error), "{error:#}");
    // Every fetch that started must unregister its tree when it ends. A
    // registration that never goes away is a leak.
    let deadline = Instant::now() + Duration::from_secs(60);
    while super::registry::active_count() != 0 {
        assert!(
            Instant::now() < deadline,
            "a fetch tree stayed registered after the fetch ended"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(unix)]
#[test]
fn ending_active_fetch_trees_stops_a_live_fetch_tree() {
    let root = tempfile::tempdir().expect("temp directory");
    let pid_file = root.path().join("interrupt.pid");
    let pid_path = path_for_stub(&pid_file);
    let heartbeat = root.path().join("interrupt.beats");
    let beats = path_for_stub(&heartbeat);
    let unix = format!(
        "#!/bin/sh\necho $$ > \"{pid_path}\"\nsh -c 'while :; do echo tick >> \"{beats}\"; sleep 0.05; done' &\nsleep 60\nexit 1\n"
    );
    let program = write_program(root.path(), "interrupt", "", &unix);
    let fetch = std::thread::spawn(move || {
        let hooks = git::empty_hooks().expect("empty hooks");
        fetch_with(
            &environment(hooks.path()),
            "file:///interrupt",
            &"0".repeat(40),
            Duration::from_secs(120),
            Some(program.as_os_str()),
            TreeStop::End,
        )
    });
    let pid = wait_for_pid(&pid_file);
    let deadline = Instant::now() + Duration::from_secs(30);
    while !super::registry::contains(pid) {
        assert!(
            Instant::now() < deadline,
            "the fetch process group was not registered"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    wait_for_heartbeat(&heartbeat);
    // Wait for a moment when this test's tree is the only registered one,
    // then keep later registrations out so the kill reaches only this tree.
    // The gate is held only for the check and the kill, never while waiting.
    let deadline = Instant::now() + Duration::from_secs(60);
    let quiet = loop {
        let quiet = super::registry::quiesce();
        if super::registry::active_count() == 1 {
            break quiet;
        }
        drop(quiet);
        assert!(
            Instant::now() < deadline,
            "other fetch trees did not end before the interrupt test"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    crate::acquisition::git::end_active_fetch_trees();
    drop(quiet);
    let error = fetch
        .join()
        .expect("fetch thread")
        .expect_err("the killed fetch must fail");
    assert!(
        !timeout_class(&error),
        "a killed tree is not a timeout: {error:#}"
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    while super::registry::contains(pid) {
        assert!(
            Instant::now() < deadline,
            "the interrupted fetch tree stayed registered"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    // The grandchild wrote a heartbeat every 50 ms. Give an in-flight write
    // time to finish, then require that no later write appears.
    std::thread::sleep(Duration::from_millis(300));
    let settled = fs::metadata(&heartbeat).map(|file| file.len()).unwrap_or(0);
    std::thread::sleep(Duration::from_millis(500));
    let later = fs::metadata(&heartbeat).map(|file| file.len()).unwrap_or(0);
    assert_eq!(
        settled, later,
        "the fetch tree kept writing heartbeats after end_active_fetch_trees"
    );
}

/// Wait for the stub to write its own process id.
#[cfg(unix)]
fn wait_for_pid(path: &Path) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(text) = fs::read_to_string(path) {
            if let Ok(pid) = text.trim().parse::<u32>() {
                return pid;
            }
        }
        assert!(
            Instant::now() < deadline,
            "the stub never wrote its process id"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Wait until the stub's grandchild has written at least one heartbeat.
#[cfg(unix)]
fn wait_for_heartbeat(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while fs::metadata(path).map(|file| file.len()).unwrap_or(0) == 0 {
        assert!(
            Instant::now() < deadline,
            "the stub never wrote a heartbeat"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
