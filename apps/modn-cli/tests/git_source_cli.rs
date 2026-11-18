#[path = "support/catalog.rs"]
mod support;

#[path = "support/git_fixture.rs"]
mod git_fixture;

#[path = "support/git_source_helpers.rs"]
mod helpers;

#[path = "support/main_bucket.rs"]
mod main_bucket;

use git_fixture::repository;
use helpers::{add_bucket, cache_archives, installed_plugin, run};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use support::{error, Fixture};

const NATIVE_DEPENDENCY_URL: &str =
    "https://github.com/fixture/lovely/releases/download/v1/native-injector.zip";

fn add_native_stack_bucket(fixture: &Fixture, root: &Path, commit: &str, dependency_pin: &str) {
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

fn seed_dependency_cache(fixture: &Fixture, archive: &Path) {
    let cache = fixture.state.join("cache");
    fs::create_dir_all(&cache).expect("cache directory");
    let url_hash = &blake3::hash(NATIVE_DEPENDENCY_URL.as_bytes()).to_hex()[..7];
    let entry = cache.join(format!("native-injector-game#lovely#1.0#{url_hash}.zip"));
    fs::copy(archive, entry).expect("seed dependency cache");
}

#[test]
fn git_source_install_uninstall_and_cache_reuse_skip_git() {
    let fixture = Fixture::new();
    let repository = repository(fixture.temp.path());
    add_bucket(&fixture, fixture.temp.path(), &repository.commit);
    fixture.register();
    let plugin = installed_plugin(&fixture);

    let installed = run(
        &fixture,
        &repository.base,
        &[
            "--json",
            "install",
            "test-game/git-package",
            "--path",
            fixture.game.to_str().expect("game path"),
            "--yes",
        ],
    );
    assert!(installed.status.success(), "{}", error(&installed));
    assert_eq!(
        fs::read(&plugin).expect("installed plugin"),
        b"pinned-release"
    );
    let archives = cache_archives(&fixture);
    assert_eq!(archives.len(), 1, "{archives:?}");

    let removed = fixture.run(&[
        "uninstall",
        "git-package",
        "--path",
        fixture.game.to_str().expect("game path"),
        "--yes",
    ]);
    assert!(removed.status.success(), "{}", error(&removed));
    assert!(!plugin.exists());

    let hidden = fixture.temp.path().join("project.git-hidden");
    fs::rename(repository.base.join("fixture/project.git"), &hidden).expect("hide the remote");
    let cached = run(
        &fixture,
        &repository.base,
        &[
            "--json",
            "install",
            "test-game/git-package",
            "--path",
            fixture.game.to_str().expect("game path"),
            "--yes",
        ],
    );
    assert!(
        cached.status.success(),
        "second install must use the cache without Git: {}",
        error(&cached)
    );
    assert_eq!(fs::read(&plugin).expect("cached plugin"), b"pinned-release");
    assert_eq!(cache_archives(&fixture).len(), 1);
}

#[test]
fn git_package_stack_downloads_a_missing_native_dependency_and_reuses_the_cache() {
    let fixture = Fixture::new();
    let repository = repository(fixture.temp.path());
    let (dependency, dependency_hash) = fixture.native_archive();
    add_native_stack_bucket(
        &fixture,
        fixture.temp.path(),
        &repository.commit,
        &dependency_hash,
    );
    seed_dependency_cache(&fixture, &dependency);
    fixture.register_native();
    let game = fixture.native_game.to_str().expect("game path");
    let plugin = fixture.native_game.join("addons/git-source.plugin");
    let library = fixture.native_game.join("winmm.dll");

    let preview = run(
        &fixture,
        &repository.base,
        &[
            "--json",
            "install",
            "native-stack-fixture/native-injector-game/steamodded",
            "--path",
            game,
            "--dry-run",
            "--download",
            "--allow-native-injector",
        ],
    );
    assert!(preview.status.success(), "{}", error(&preview));
    let report: Value = serde_json::from_slice(&preview.stdout).expect("dry-run JSON");
    assert_eq!(report["plan"]["status"], "reviewable");
    assert_eq!(
        report["dependencies"][0]["reference"],
        "native-stack-fixture/native-injector-game/lovely"
    );
    assert!(!plugin.exists());
    assert!(!library.exists());
    assert_eq!(cache_archives(&fixture).len(), 2);

    // Drop the Git archive so the stack install itself must acquire it and
    // record its digest; the preview above leaves it warm in the cache.
    let dropped = fixture.run(&["cache", "rm", "steamodded"]);
    assert!(dropped.status.success(), "{}", error(&dropped));
    assert_eq!(cache_archives(&fixture).len(), 1);

    let installed = run(
        &fixture,
        &repository.base,
        &[
            "--json",
            "install",
            "native-stack-fixture/native-injector-game/steamodded",
            "--path",
            game,
            "--yes",
            "--allow-native-injector",
        ],
    );
    assert!(installed.status.success(), "{}", error(&installed));
    let report: Value = serde_json::from_slice(&installed.stdout).expect("install JSON");
    let order: Vec<&str> = report["stack"]
        .as_array()
        .expect("stack")
        .iter()
        .map(|member| member["package"].as_str().expect("reference"))
        .collect();
    assert_eq!(
        order,
        [
            "native-stack-fixture/native-injector-game/lovely",
            "native-stack-fixture/native-injector-game/steamodded"
        ]
    );
    assert_eq!(
        fs::read(&plugin).expect("installed plugin"),
        b"pinned-release"
    );
    assert_eq!(
        fs::read(&library).expect("native library"),
        b"native-injector-payload"
    );

    let removed = fixture.run_paths(
        &[
            "uninstall",
            "steamodded",
            "--include-deps",
            "--yes",
            "--path",
        ],
        &[&fixture.native_game],
    );
    assert!(removed.status.success(), "{}", error(&removed));
    assert!(!plugin.exists());
    assert!(!library.exists());

    let hidden = fixture.temp.path().join("project.git-hidden");
    fs::rename(repository.base.join("fixture/project.git"), &hidden).expect("hide the remote");
    let cached = run(
        &fixture,
        &repository.base,
        &[
            "--json",
            "install",
            "native-stack-fixture/native-injector-game/steamodded",
            "--path",
            game,
            "--yes",
            "--allow-native-injector",
        ],
    );
    assert!(
        cached.status.success(),
        "second stack install must use the cache without Git: {}",
        error(&cached)
    );
    assert_eq!(fs::read(&plugin).expect("cached plugin"), b"pinned-release");
    assert_eq!(
        fs::read(&library).expect("cached native library"),
        b"native-injector-payload"
    );
    assert_eq!(cache_archives(&fixture).len(), 2);
}

#[test]
fn git_source_dry_run_without_download_reports_needed_download_without_git() {
    let fixture = Fixture::new();
    let repository = repository(fixture.temp.path());
    add_bucket(&fixture, fixture.temp.path(), &repository.commit);
    fixture.register();
    let missing_remote = fixture.temp.path().join("no-such-remote");

    let preview = run(
        &fixture,
        &missing_remote,
        &[
            "--json",
            "install",
            "test-game/git-package",
            "--path",
            fixture.game.to_str().expect("game path"),
            "--dry-run",
        ],
    );
    assert!(preview.status.success(), "{}", error(&preview));
    let report: Value = serde_json::from_slice(&preview.stdout).expect("dry-run JSON");
    let needed = report["archive_needed"]
        .as_str()
        .expect("dry run names the needed download");
    assert!(needed.contains("download"), "{needed}");
    assert!(needed.contains(&repository.commit), "{needed}");
    assert!(!installed_plugin(&fixture).exists());
    assert!(cache_archives(&fixture).is_empty());
}

#[test]
fn git_source_missing_commit_fails_with_code_and_no_writes() {
    let fixture = Fixture::new();
    let repository = repository(fixture.temp.path());
    add_bucket(&fixture, fixture.temp.path(), &"0".repeat(40));
    fixture.register();
    let state_db = fixture.state.join("state.sqlite");
    let state_before = fs::read(&state_db).expect("state bytes");
    let game_before = fixture.game.join("bin/TestGame.exe");
    let marker = fs::read(&game_before).expect("game marker");

    let failed = run(
        &fixture,
        &repository.base,
        &[
            "--json",
            "install",
            "test-game/git-package",
            "--path",
            fixture.game.to_str().expect("game path"),
            "--yes",
        ],
    );
    assert!(!failed.status.success());
    let envelope: Value = serde_json::from_slice(&failed.stderr).expect("JSON error envelope");
    assert_eq!(envelope["error"], "GIT_SOURCE_FETCH_FAILED");
    assert!(!fixture.game.join("addons").exists());
    assert_eq!(fs::read(&game_before).expect("game marker"), marker);
    assert_eq!(fs::read(&state_db).expect("state bytes"), state_before);
    assert!(cache_archives(&fixture).is_empty());
}

/// Write a bucket whose first Git source stalls and whose second one serves.
fn add_stalled_then_serving_bucket(fixture: &Fixture, root: &Path, commit: &str) {
    let bucket = root.join("git-stall-fixture");
    let game = bucket.join("mods/test-game");
    fs::create_dir_all(&game).expect("bucket directory");
    fs::write(
        bucket.join("bucket.json"),
        br#"{"schema_version":2,"name":"git-stall-fixture","title":"Git stall fixture"}"#,
    )
    .expect("bucket metadata");
    let manifest = json!({
        "schema_version": 2,
        "id": "git-package",
        "game": "test-game",
        "name": "Git Package",
        "version": "1.0",
        "sources": [
            {"kind": "git", "repository": "fixture/stalled", "commit": commit},
            {"kind": "git", "repository": "fixture/project", "commit": commit},
        ],
    });
    fs::write(
        game.join("git-package.json"),
        serde_json::to_vec(&manifest).expect("manifest JSON"),
    )
    .expect("manifest");
    let result = fixture.run_paths(&["bucket", "add", "git-stall-fixture"], &[&bucket]);
    assert!(result.status.success(), "{}", error(&result));
}

/// The real Git program on PATH, called through by the stalling stub.
fn real_git() -> std::path::PathBuf {
    let name = if cfg!(windows) { "git.exe" } else { "git" };
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
        .expect("the fixture needs Git on PATH")
}

/// A stub Git that stalls for `fixture/stalled` and calls the real Git otherwise.
fn stalling_git_program(root: &Path) -> std::path::PathBuf {
    let directory = root.join("stalled-git");
    fs::create_dir_all(&directory).expect("stall stub directory");
    let real = real_git();
    let real = real.to_string_lossy().replace(char::from(92), "/");
    if cfg!(windows) {
        let program = directory.join("git.bat");
        let script = format!(
            "@echo off\r\necho %* | findstr /C:\"fixture/stalled\" >nul\r\nif errorlevel 1 goto real\r\nping -n 60 127.0.0.1 >nul\r\nexit /b 0\r\n:real\r\n\"{real}\" %*\r\n"
        );
        fs::write(&program, script).expect("stall stub");
        program
    } else {
        let program = directory.join("git");
        let script = format!(
            "#!/bin/sh\ncase \"$*\" in\n  *fixture/stalled*) sleep 60 ;;\n  *) exec \"{real}\" \"$@\" ;;\nesac\n"
        );
        fs::write(&program, script).expect("stall stub");
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

/// Run the CLI with a stubbed Git fetch and a 2 s fetch deadline.
fn run_with_stalled_git(fixture: &Fixture, base: &Path, args: &[&str]) -> std::process::Output {
    let program = stalling_git_program(fixture.temp.path());
    main_bucket::command(env!("CARGO_BIN_EXE_modn"))
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .env("MODN_TEST_GIT_REMOTE_BASE", git_fixture::file_url(base))
        .env("MODN_TEST_GIT_FETCH_PROGRAM", &program)
        .env("MODN_TEST_GIT_FETCH_TIMEOUT_MS", "2000")
        .args(args)
        .output()
        .expect("run CLI")
}

#[test]
fn a_stalled_first_git_fetch_times_out_and_the_next_source_serves() {
    let fixture = Fixture::new();
    let repository = repository(fixture.temp.path());
    add_stalled_then_serving_bucket(&fixture, fixture.temp.path(), &repository.commit);
    fixture.register();
    let plugin = installed_plugin(&fixture);
    let started = std::time::Instant::now();

    let output = run_with_stalled_git(
        &fixture,
        &repository.base,
        &[
            "install",
            "test-game/git-package",
            "--path",
            fixture.game.to_str().expect("game path"),
            "--yes",
        ],
    );

    assert!(output.status.success(), "{}", error(&output));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(20),
        "the stalled fetch blocked the ordered fallback: {:?}",
        started.elapsed()
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr
            .lines()
            .any(|line| line.contains("git fixture/stalled commit ")
                && line.contains("attempt 1: timeout")),
        "the report does not classify the stall as a timeout: {stderr}"
    );
    assert!(
        stderr
            .lines()
            .any(|line| line.contains("git fixture/project commit ")
                && line.trim_end().ends_with(": ok")),
        "the report omits the serving Git source: {stderr}"
    );
    assert_eq!(fs::read(&plugin).expect("installed plugin"), b"pinned-release");
}
