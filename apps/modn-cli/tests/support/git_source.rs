#![allow(
    dead_code,
    reason = "each integration-test crate uses a different subset of these helpers"
)]
use crate::git_fixture;
use crate::main_bucket;
use crate::support::{error, Fixture};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

pub const NATIVE_DEPENDENCY_URL: &str =
    "https://github.com/fixture/lovely/releases/download/v1/native-injector.zip";

pub fn add_native_stack_bucket(fixture: &Fixture, root: &Path, commit: &str, dependency_pin: &str) {
    let bucket = root.join("native-stack-fixture");
    let game = bucket.join("mods/native-injector-game");
    fs::create_dir_all(&game).expect("bucket directory");
    fs::write(
        bucket.join("bucket.json"),
        br#"{"schema_version":2,"name":"native-stack-fixture","title":"Native stack fixture"}"#,
    )
    .expect("bucket metadata");
    let packages = [
        json!({
            "schema_version": 2,
            "id": "steamodded",
            "game": "native-injector-game",
            "name": "Git Stack Package",
            "version": "1.0",
            "sources": [{"kind": "git", "repository": "fixture/project", "commit": commit}],
            "dependencies": ["lovely"],
        }),
        json!({
            "schema_version": 2,
            "id": "lovely",
            "game": "native-injector-game",
            "name": "Native Dependency",
            "version": "1.0",
            "archive_format": "zip",
            "blake3": dependency_pin,
            "sources": [
                {
                    "kind": "github-release",
                    "repository": "fixture/lovely",
                    "tag": "v1",
                    "asset": "native-injector.zip",
                },
            ],
            "native_injector": true,
        }),
    ];
    for manifest in packages {
        let id = manifest["id"].as_str().expect("package id");
        fs::write(
            game.join(format!("{id}.json")),
            serde_json::to_vec(&manifest).expect("manifest JSON"),
        )
        .expect("manifest");
    }
    let result = fixture.run_paths(&["bucket", "add", "native-stack-fixture"], &[&bucket]);
    assert!(result.status.success(), "{}", error(&result));
}

pub fn seed_dependency_cache(fixture: &Fixture, archive: &Path) {
    let cache = fixture.state.join("cache");
    fs::create_dir_all(&cache).expect("cache directory");
    let url_hash = &blake3::hash(NATIVE_DEPENDENCY_URL.as_bytes()).to_hex()[..7];
    let entry = cache.join(format!("native-injector-game#lovely#1.0#{url_hash}.zip"));
    fs::copy(archive, entry).expect("seed dependency cache");
}

/// Write a bucket whose first Git source stalls and whose second one serves.
pub fn add_stalled_then_serving_bucket(fixture: &Fixture, root: &Path, commit: &str) {
    add_two_source_bucket(fixture, root, "git-stall-fixture", "stalled", commit);
}

/// Write a bucket whose first Git source aborts below the Git low-speed limit
/// and whose second one serves.
pub fn add_low_speed_then_serving_bucket(fixture: &Fixture, root: &Path, commit: &str) {
    add_two_source_bucket(fixture, root, "git-low-speed-fixture", "low-speed", commit);
}

/// Write a bucket whose first source names `first` and whose second serves.
pub fn add_two_source_bucket(
    fixture: &Fixture,
    root: &Path,
    bucket_name: &str,
    first: &str,
    commit: &str,
) {
    write_two_source_bucket(fixture, root, bucket_name, first, "1.0", commit, commit);
}

/// Write a two-Git-source release with explicit version and commits.
///
/// The first source names `fixture/{first}`; the second names the serving
/// `fixture/project`. `first_commit` lets the fallback source pin a different
/// commit than the primary, as a fallback release can.
pub fn write_two_source_bucket(
    fixture: &Fixture,
    root: &Path,
    bucket_name: &str,
    first: &str,
    version: &str,
    first_commit: &str,
    second_commit: &str,
) {
    write_two_source_manifest(
        root,
        bucket_name,
        first,
        version,
        first_commit,
        second_commit,
    );
    let bucket = root.join(bucket_name);
    let result = fixture.run_paths(&["bucket", "add", bucket_name], &[&bucket]);
    assert!(result.status.success(), "{}", error(&result));
}

/// Rewrite a two-Git-source release in place, without registering it again.
pub fn write_two_source_manifest(
    root: &Path,
    bucket_name: &str,
    first: &str,
    version: &str,
    first_commit: &str,
    second_commit: &str,
) {
    let bucket = root.join(bucket_name);
    let game = bucket.join("mods/test-game");
    fs::create_dir_all(&game).expect("bucket directory");
    fs::write(
        bucket.join("bucket.json"),
        format!(
            r#"{{"schema_version":2,"name":"{bucket_name}","title":"Two-source Git fixture"}}"#
        ),
    )
    .expect("bucket metadata");
    let manifest = json!({
        "schema_version": 2,
        "id": "git-package",
        "game": "test-game",
        "name": "Git Package",
        "version": version,
        "sources": [
            {"kind": "git", "repository": format!("fixture/{first}"), "commit": first_commit},
            {"kind": "git", "repository": "fixture/project", "commit": second_commit},
        ],
    });
    fs::write(
        game.join("git-package.json"),
        serde_json::to_vec(&manifest).expect("manifest JSON"),
    )
    .expect("manifest");
}

/// The real Git program on PATH, called through by the stalling stub.
pub fn real_git() -> PathBuf {
    let name = if cfg!(windows) { "git.exe" } else { "git" };
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
        .expect("the fixture needs Git on PATH")
}

/// A stub Git that stalls for `fixture/stalled` and calls the real Git otherwise.
pub fn stalling_git_program(root: &Path) -> PathBuf {
    stub_git_program(
        root,
        "stalled-git",
        "fixture/stalled",
        "ping -n 60 127.0.0.1 >nul",
        "sleep 60",
    )
}

/// A stub Git that reports the low-speed abort for `fixture/low-speed` and
/// calls the real Git otherwise.
pub fn low_speed_git_program(root: &Path) -> PathBuf {
    stub_git_program(
        root,
        "low-speed-git",
        "fixture/low-speed",
        "echo error: RPC failed; curl 28 Operation too slow. Less than 1 bytes/sec transferred the last 60 seconds 1>&2\r\nexit /b 128",
        "echo \"error: RPC failed; curl 28 Operation too slow. Less than 1 bytes/sec transferred the last 60 seconds\" >&2\nexit 128",
    )
}

/// Write a stub Git that handles the matching repository and forwards the rest.
pub fn stub_git_program(
    root: &Path,
    name: &str,
    marker: &str,
    windows_action: &str,
    unix_action: &str,
) -> PathBuf {
    let directory = root.join(name);
    fs::create_dir_all(&directory).expect("stub directory");
    let real = real_git();
    let real = real.to_string_lossy().replace(char::from(92), "/");
    if cfg!(windows) {
        let program = directory.join("git.bat");
        let script = format!(
            "@echo off\r\necho %* | findstr /C:\"{marker}\" >nul\r\nif errorlevel 1 goto real\r\n{windows_action}\r\n:real\r\n\"{real}\" %*\r\n"
        );
        fs::write(&program, script).expect("stub program");
        program
    } else {
        let program = directory.join("git");
        let script = format!(
            "#!/bin/sh\ncase \"$*\" in\n  *{marker}*)\n    {unix_action}\n    ;;\n  *)\n    exec \"{real}\" \"$@\"\n    ;;\nesac\n"
        );
        fs::write(&program, script).expect("stub program");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(&program).expect("stub metadata").permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&program, permissions).expect("stub permissions");
        }
        program
    }
}

/// Run the CLI with a stubbed Git fetch and a short inactivity limit.
///
/// The stderr stream goes to a file and the helper reports when the process
/// exited. A killed batch stub can leave a descendant that holds a pipe open,
/// so a pipe capture would measure the descendant, not the command.
pub fn run_with_stub_git(
    fixture: &Fixture,
    base: &Path,
    args: &[&str],
    program: &Path,
    idle_millis: &str,
) -> (Output, std::time::Duration) {
    run_with_stub_git_env(fixture, base, args, program, idle_millis, &[])
}

/// Run the CLI with a stubbed Git fetch, extra environment entries, and a
/// short inactivity limit.
pub fn run_with_stub_git_env(
    fixture: &Fixture,
    base: &Path,
    args: &[&str],
    program: &Path,
    idle_millis: &str,
    environment: &[(&str, &str)],
) -> (Output, std::time::Duration) {
    let capture = fixture.temp.path().join("stubbed-stderr.txt");
    let stderr = fs::File::create(&capture).expect("stderr capture");
    let mut command = main_bucket::command(env!("CARGO_BIN_EXE_modn"));
    command
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .env("MODN_TEST_GIT_REMOTE_BASE", git_fixture::file_url(base))
        .env("MODN_TEST_GIT_FETCH_PROGRAM", program)
        .env("MODN_TEST_GIT_FETCH_IDLE_MS", idle_millis)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(stderr);
    for (name, value) in environment {
        command.env(name, value);
    }
    let mut child = command.spawn().expect("run CLI");
    let started = std::time::Instant::now();
    let status = child.wait().expect("wait for CLI");
    (
        Output {
            status,
            stdout: Vec::new(),
            stderr: fs::read(&capture).expect("captured stderr"),
        },
        started.elapsed(),
    )
}

/// Run the CLI with a stubbed Git fetch and capture both output streams.
///
/// A dry-run report goes to stdout, so a test that inspects the JSON or the
/// text report reads it from the capture files instead of a pipe.
pub fn run_with_stub_git_captured(
    fixture: &Fixture,
    base: &Path,
    args: &[&str],
    program: &Path,
    idle_millis: &str,
) -> (Output, std::time::Duration) {
    let stdout_path = fixture.temp.path().join("stubbed-stdout.txt");
    let stdout = fs::File::create(&stdout_path).expect("stdout capture");
    let stderr_path = fixture.temp.path().join("stubbed-stderr.txt");
    let stderr = fs::File::create(&stderr_path).expect("stderr capture");
    let mut command = main_bucket::command(env!("CARGO_BIN_EXE_modn"));
    command
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .env("MODN_TEST_GIT_REMOTE_BASE", git_fixture::file_url(base))
        .env("MODN_TEST_GIT_FETCH_PROGRAM", program)
        .env("MODN_TEST_GIT_FETCH_IDLE_MS", idle_millis)
        .args(args)
        .stdout(stdout)
        .stderr(stderr);
    let mut child = command.spawn().expect("run CLI");
    let started = std::time::Instant::now();
    let status = child.wait().expect("wait for CLI");
    (
        Output {
            status,
            stdout: fs::read(&stdout_path).expect("captured stdout"),
            stderr: fs::read(&stderr_path).expect("captured stderr"),
        },
        started.elapsed(),
    )
}

/// Staging directories that a fetch should have removed.
pub fn leftover_fetch_directories(fixture: &Fixture) -> Vec<String> {
    let cache = fixture.state.join("cache");
    let Ok(entries) = fs::read_dir(cache) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("git-source-"))
        .collect();
    names.sort();
    names
}
