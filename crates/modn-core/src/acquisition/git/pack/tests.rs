use super::*;
use crate::acquisition::{ArchiveCache, CacheUse, GitCacheKey, GitSource};
use crate::git::{self, GitEnvironment};
use crate::progress::{CancellationToken, OperationControl};
use std::net::TcpListener;
use std::path::Path;

fn token() -> CancellationToken {
    CancellationToken::new()
}

/// The most bytes an error message may add to the kept tail.
const MESSAGE_OVERHEAD: usize = 1024;

fn environment<'a>(directory: &'a Path, hooks: &'a Path) -> GitEnvironment<'a> {
    GitEnvironment {
        directory,
        hooks,
        allow_file: true,
    }
}

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

/// Write a stand-in Git program that prints the preamble, then `megabytes` of
/// `remote:` messages to its error stream, then one last line, and exits 1.
fn flooding_program(root: &Path, preamble: &[&str], megabytes: usize) -> std::path::PathBuf {
    let flood = root.join("flood.txt");
    let line =
        "remote: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let mut text = String::with_capacity(1024 * 1024 + line.len());
    while text.len() < 1024 * 1024 {
        text.push_str(line);
        text.push('\n');
    }
    std::fs::write(&flood, &text).expect("flood file");
    let flood = flood.to_string_lossy().replace('\\', "/");
    let directory = root.join("flooding-git");
    std::fs::create_dir_all(&directory).expect("program directory");
    if cfg!(windows) {
        let mut script = String::from("@echo off\r\n");
        for line in preamble {
            script.push_str(&format!("echo {line} 1>&2\r\n"));
        }
        for _ in 0..megabytes {
            script.push_str(&format!("type \"{flood}\" 1>&2\r\n"));
        }
        script.push_str("echo fatal: stub tail 9f3a 1>&2\r\nexit /b 1\r\n");
        let program = directory.join("git.bat");
        std::fs::write(&program, script).expect("program");
        program
    } else {
        let mut script = String::from("#!/bin/sh\n");
        for line in preamble {
            script.push_str(&format!("printf '%s\\n' '{line}' >&2\n"));
        }
        script.push_str("i=0\n");
        script.push_str(&format!(
            "while [ \"$i\" -lt {megabytes} ]; do cat \"{flood}\" >&2; i=$((i+1)); done\n"
        ));
        script.push_str("printf '%s\\n' 'fatal: stub tail 9f3a' >&2\nexit 1\n");
        let program = directory.join("git");
        std::fs::write(&program, script).expect("program");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&program)
                .expect("program metadata")
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&program, permissions).expect("program permissions");
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
fn the_shared_git_command_passes_the_low_speed_limits() {
    let hooks = git::empty_hooks().expect("empty hooks");
    let command = git::command(&environment(hooks.path(), hooks.path()));
    let arguments: Vec<String> = command
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();
    assert!(
        arguments
            .windows(2)
            .any(|pair| pair == ["-c", "http.lowSpeedLimit=1"]),
        "the shared command omits the low-speed limit: {arguments:?}"
    );
    let expected = format!("http.lowSpeedTime={}", git::low_speed_time_seconds());
    assert!(
        arguments
            .windows(2)
            .any(|pair| pair[0] == "-c" && pair[1] == expected),
        "the shared command omits {expected}: {arguments:?}"
    );
    assert_eq!(
        git::default_low_speed_time_seconds(),
        90,
        "the low-speed time must match the documented inactivity limit"
    );
}

#[test]
fn a_real_low_speed_abort_keeps_the_timeout_class() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("stalling TCP listener");
    let port = listener.local_addr().expect("listener address").port();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for connection in listener.incoming() {
            match connection {
                Ok(connection) => held.push(connection),
                Err(_) => break,
            }
        }
    });

    let root = tempfile::tempdir().expect("temp directory");
    let hooks = git::empty_hooks().expect("empty hooks");
    let environment = environment(root.path(), hooks.path());
    git::run(&environment, &["init", "--bare", "--quiet"]).expect("bare repository");

    let url = format!("http://127.0.0.1:{port}/repo.git");
    let commit = "0".repeat(40);
    let command = git::command_with_low_speed_time(&environment, 1);
    let error = fetch_with_command(command, &url, &commit, &token())
        .expect_err("a stalled HTTP transfer must fail");
    let text = format!("{error:#}");
    assert!(timeout_class(&error), "{text}");
    assert!(
        text.contains("Operation too slow"),
        "the test must use Git's real low-speed text: {text}"
    );
}

#[test]
fn a_fetch_failure_without_low_speed_text_is_not_a_timeout() {
    let root = tempfile::tempdir().expect("temp directory");
    let hooks = git::empty_hooks().expect("empty hooks");
    let environment = environment(root.path(), hooks.path());
    git::run(&environment, &["init", "--bare", "--quiet"]).expect("bare repository");
    let missing = root.path().join("missing.git");
    let missing = missing.to_str().expect("missing repository path");
    let error = fetch_with(&environment, missing, &"0".repeat(40), &token())
        .expect_err("a missing repository must fail");
    let text = format!("{error:#}");
    assert!(!timeout_class(&error), "{text}");
    assert!(text.contains("failed"), "{text}");
}

#[test]
fn a_flood_of_git_error_output_keeps_only_the_tail_and_bounds_the_message() {
    let root = tempfile::tempdir().expect("temp directory");
    let program = flooding_program(root.path(), &[], 16);
    let _program = Program::install(&program);
    let hooks = git::empty_hooks().expect("empty hooks");
    let environment = environment(root.path(), hooks.path());
    let commit = "a".repeat(40);
    let error = fetch_with_command(
        git::command(&environment),
        "https://example.invalid/owner/project.git",
        &commit,
        &token(),
    )
    .expect_err("a stub Git that exits non-zero must fail the fetch");
    let text = format!("{error:#}");
    assert!(!timeout_class(&error), "{text}");
    assert!(text.contains("GIT_SOURCE_FETCH_FAILED"), "{text}");
    assert!(
        text.contains("showing the kept tail"),
        "the message must say the output was truncated: {text}"
    );
    assert!(
        text.contains("fatal: stub tail 9f3a"),
        "the message must hold the last error line; message length {}",
        text.len()
    );
    assert!(
        text.len() <= super::super::run::OUTPUT_TAIL_LIMIT + MESSAGE_OVERHEAD,
        "the message grew past the bound: {} bytes",
        text.len()
    );
}

#[test]
fn a_truncated_low_speed_abort_keeps_the_timeout_class() {
    let root = tempfile::tempdir().expect("temp directory");
    let program = flooding_program(
        root.path(),
        &[
            "fatal: unable to access 'https://example.invalid/': Operation too slow. Less than 1 bytes/sec transferred the last 1 seconds",
        ],
        16,
    );
    let _program = Program::install(&program);
    let hooks = git::empty_hooks().expect("empty hooks");
    let environment = environment(root.path(), hooks.path());
    let commit = "b".repeat(40);
    let error = fetch_with_command(
        git::command(&environment),
        "https://example.invalid/owner/project.git",
        &commit,
        &token(),
    )
    .expect_err("the stub Git exits non-zero after the low-speed line");
    let text = format!("{error:#}");
    assert!(timeout_class(&error), "{text}");
    assert!(
        text.contains("stopped below the Git low-speed limit"),
        "{text}"
    );
    assert!(
        text.contains("showing the kept tail"),
        "the message must say the output was truncated: {text}"
    );
    assert!(
        text.len() <= super::super::run::OUTPUT_TAIL_LIMIT + MESSAGE_OVERHEAD,
        "the message grew past the bound: {} bytes",
        text.len()
    );
}

/// A stand-in Git program that blocks on one matching command and forwards
/// every other command to the real Git.
///
/// The blocking branch runs inside this process and starts no child, so
/// stopping it closes its pipes. It writes `started` before it blocks and
/// appends to `heartbeat` while it blocks.
struct BlockingStub {
    program: std::path::PathBuf,
    started: std::path::PathBuf,
    heartbeat: std::path::PathBuf,
}

impl BlockingStub {
    fn install(root: &Path, marker: &str) -> Self {
        let directory = root.join("blocking-git");
        std::fs::create_dir_all(&directory).expect("program directory");
        let started = directory.join("started");
        let heartbeat = directory.join("heartbeat");
        let real = real_git().to_string_lossy().replace('\\', "/");
        let started_text = started.to_string_lossy().replace('\\', "/");
        let heartbeat_text = heartbeat.to_string_lossy().replace('\\', "/");
        let program = if cfg!(windows) {
            let program = directory.join("git.bat");
            let script = format!(
                "@echo off\r\necho %* | findstr /C:\"{marker}\" >nul\r\nif errorlevel 1 goto real\r\n>\"{started_text}\" echo started\r\n:wait\r\n>>\"{heartbeat_text}\" echo .\r\ngoto wait\r\n:real\r\n\"{real}\" %*\r\n"
            );
            std::fs::write(&program, script).expect("program");
            program
        } else {
            let program = directory.join("git");
            let script = format!(
                "#!/bin/sh\ncase \"$*\" in\n  *{marker}*)\n    : > \"{started_text}\"\n    while :; do echo . >> \"{heartbeat_text}\"; done\n    ;;\n  *)\n    exec \"{real}\" \"$@\"\n    ;;\nesac\n"
            );
            std::fs::write(&program, script).expect("program");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut permissions = std::fs::metadata(&program)
                    .expect("program metadata")
                    .permissions();
                permissions.set_mode(0o755);
                std::fs::set_permissions(&program, permissions).expect("program permissions");
            }
            program
        };
        Self {
            program,
            started,
            heartbeat,
        }
    }

    /// Whether the stub added to its heartbeat during a short observation.
    fn still_running(&self) -> bool {
        let before = std::fs::metadata(&self.heartbeat).map_or(0, |metadata| metadata.len());
        std::thread::sleep(std::time::Duration::from_millis(200));
        std::fs::metadata(&self.heartbeat).map_or(0, |metadata| metadata.len()) > before
    }
}

fn real_git() -> std::path::PathBuf {
    let name = if cfg!(windows) { "git.exe" } else { "git" };
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
        .expect("the test needs Git on PATH")
}

fn wait_for_marker(path: &Path, limit: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + limit;
    while !path.exists() {
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    true
}

fn is_cancelled(error: &anyhow::Error) -> bool {
    crate::error::code_of(error) == Some(crate::error::ErrorCode::Cancelled)
}

fn cache_snapshot(cache: &ArchiveCache) -> Vec<(String, Vec<u8>)> {
    let mut snapshot: Vec<(String, Vec<u8>)> = cache
        .entries(None)
        .expect("cache entries")
        .into_iter()
        .map(|entry| {
            let name = entry
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let bytes = std::fs::read(&entry.path).expect("cache entry bytes");
            (name, bytes)
        })
        .collect();
    snapshot.sort();
    snapshot
}

/// A cache with one previous Git entry and the commit each test pins.
fn cached_fixture(root: &Path) -> (ArchiveCache, String) {
    let cache = ArchiveCache::new(root.join("cache"));
    std::fs::create_dir_all(cache.root()).expect("cache directory");
    let commit = "a".repeat(40);
    let key = GitCacheKey::new("test-game", "fixture", "1.0", &commit);
    std::fs::write(cache.git_path(&key, &"c".repeat(64)), b"previous archive")
        .expect("previous cache entry");
    (cache, commit)
}

const SLOW_REMOTE: &str = "https://example.invalid/fixture/project.git";

#[test]
fn a_cancel_during_a_slow_fetch_stops_the_git_child_and_keeps_the_cache() {
    let root = tempfile::tempdir().expect("temp directory");
    let stub = BlockingStub::install(root.path(), "example.invalid");
    let _program = Program::install(&stub.program);
    let (cache, commit) = cached_fixture(root.path());
    let key = GitCacheKey::new("test-game", "fixture", "1.0", &commit);
    let before = cache_snapshot(&cache);
    let source = GitSource::new("fixture/project".into(), commit.clone());
    let token = CancellationToken::new();
    let control = OperationControl::new(token.clone(), |_event| {});
    let clock = std::time::Instant::now();
    let watcher = std::thread::spawn({
        let marker = stub.started.clone();
        let token = token.clone();
        move || {
            let started_on_time = wait_for_marker(&marker, std::time::Duration::from_secs(30));
            token.cancel();
            started_on_time
        }
    });
    let error = source
        .acquire_url(SLOW_REMOTE, &cache, &key, CacheUse::Reuse, &control)
        .expect_err("a cancelled fetch must fail");
    let elapsed = clock.elapsed();
    let started_on_time = watcher.join().expect("watcher thread");
    let text = format!("{error:#}");
    assert!(started_on_time, "the stub Git did not start within 30 s");
    assert!(is_cancelled(&error), "{text}");
    assert!(!text.contains("GIT_SOURCE_FETCH_FAILED"), "{text}");
    assert!(
        elapsed < std::time::Duration::from_secs(20),
        "the cancel took {elapsed:?}, not the bounded poll plus kill"
    );
    assert!(
        !stub.still_running(),
        "the stub Git still runs after the cancel"
    );
    assert_eq!(
        cache_snapshot(&cache),
        before,
        "the cancelled fetch changed the cache"
    );
}

#[test]
fn a_cancel_before_the_spawn_starts_no_git_child() {
    let root = tempfile::tempdir().expect("temp directory");
    // A program that cannot be spawned proves that no child started: a spawn
    // attempt fails with the missing-program error, not with CANCELLED.
    let absent = root
        .path()
        .join("absent-git")
        .join(if cfg!(windows) { "git.bat" } else { "git" });
    let _program = Program::install(&absent);
    let (cache, commit) = cached_fixture(root.path());
    let key = GitCacheKey::new("test-game", "fixture", "1.0", &commit);
    let before = cache_snapshot(&cache);
    let source = GitSource::new("fixture/project".into(), commit.clone());
    let token = CancellationToken::new();
    token.cancel();
    let control = OperationControl::new(token, |_event| {});
    let error = source
        .acquire_url(SLOW_REMOTE, &cache, &key, CacheUse::Reuse, &control)
        .expect_err("a cancelled operation must fail before it spawns Git");
    let text = format!("{error:#}");
    assert!(is_cancelled(&error), "{text}");
    assert_eq!(
        cache_snapshot(&cache),
        before,
        "the cancelled operation changed the cache"
    );
}

#[test]
fn a_cancel_during_a_slow_catalog_clone_stops_the_git_child() {
    let root = tempfile::tempdir().expect("temp directory");
    let stub = BlockingStub::install(root.path(), "example.invalid");
    let _program = Program::install(&stub.program);
    let destination = root.path().join("catalog");
    std::fs::create_dir_all(&destination).expect("catalog directory");
    let token = CancellationToken::new();
    let clock = std::time::Instant::now();
    let watcher = std::thread::spawn({
        let marker = stub.started.clone();
        let token = token.clone();
        move || {
            let started_on_time = wait_for_marker(&marker, std::time::Duration::from_secs(30));
            token.cancel();
            started_on_time
        }
    });
    let error = clone_catalog(
        &destination,
        "https://example.invalid/fixture/catalog.git",
        &token,
    )
    .expect_err("a cancelled clone must fail");
    let elapsed = clock.elapsed();
    let started_on_time = watcher.join().expect("watcher thread");
    let text = format!("{error:#}");
    assert!(started_on_time, "the stub Git did not start within 30 s");
    assert!(is_cancelled(&error), "{text}");
    assert!(!text.contains("GIT_CATALOG_FETCH_FAILED"), "{text}");
    assert!(
        elapsed < std::time::Duration::from_secs(20),
        "the cancel took {elapsed:?}, not the bounded poll plus kill"
    );
    assert!(
        !stub.still_running(),
        "the stub Git still runs after the cancel"
    );
}
