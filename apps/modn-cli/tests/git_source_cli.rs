#[path = "support/catalog.rs"]
mod support;

#[path = "support/git_fixture.rs"]
mod git_fixture;

#[path = "support/git_source.rs"]
mod git_source;

#[path = "support/git_source_helpers.rs"]
mod helpers;

#[path = "support/main_bucket.rs"]
mod main_bucket;

use git_fixture::repository;
use git_source::{
    add_low_speed_then_serving_bucket, add_native_stack_bucket, add_stalled_then_serving_bucket,
    leftover_fetch_directories, low_speed_git_program, run_with_stub_git, seed_dependency_cache,
    stalling_git_program,
};
use helpers::{add_bucket, cache_archives, installed_plugin, run};
use serde_json::Value;
use std::fs;
use support::{error, Fixture};

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

#[test]
fn a_stalled_first_git_fetch_times_out_and_the_next_source_serves() {
    let fixture = Fixture::new();
    let repository = repository(fixture.temp.path());
    add_stalled_then_serving_bucket(&fixture, fixture.temp.path(), &repository.commit);
    fixture.register();
    let plugin = installed_plugin(&fixture);
    let program = stalling_git_program(fixture.temp.path());

    let (output, elapsed) = run_with_stub_git(
        &fixture,
        &repository.base,
        &[
            "install",
            "test-game/git-package",
            "--path",
            fixture.game.to_str().expect("game path"),
            "--yes",
        ],
        &program,
        "8000",
    );

    assert!(output.status.success(), "{}", error(&output));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        elapsed < std::time::Duration::from_secs(30),
        "the stalled fetch blocked the ordered fallback: {elapsed:?}\n{stderr}"
    );
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
    assert_eq!(
        fs::read(&plugin).expect("installed plugin"),
        b"pinned-release"
    );
    assert_eq!(
        leftover_fetch_directories(&fixture),
        Vec::<String>::new(),
        "the timed-out fetch left its staging directory behind"
    );
}

#[test]
fn a_low_speed_abort_is_a_timeout_and_the_next_source_serves() {
    let fixture = Fixture::new();
    let repository = repository(fixture.temp.path());
    add_low_speed_then_serving_bucket(&fixture, fixture.temp.path(), &repository.commit);
    fixture.register();
    let plugin = installed_plugin(&fixture);
    let program = low_speed_git_program(fixture.temp.path());

    let (output, elapsed) = run_with_stub_git(
        &fixture,
        &repository.base,
        &[
            "install",
            "test-game/git-package",
            "--path",
            fixture.game.to_str().expect("game path"),
            "--yes",
        ],
        &program,
        "30000",
    );

    assert!(output.status.success(), "{}", error(&output));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        elapsed < std::time::Duration::from_secs(30),
        "the low-speed abort blocked the ordered fallback: {elapsed:?}\n{stderr}"
    );
    assert!(
        stderr
            .lines()
            .any(|line| line.contains("git fixture/low-speed commit ")
                && line.contains("attempt 1: timeout")),
        "the report does not classify the low-speed abort as a timeout: {stderr}"
    );
    assert!(
        stderr
            .lines()
            .any(|line| line.contains("git fixture/project commit ")
                && line.trim_end().ends_with(": ok")),
        "the report omits the serving Git source: {stderr}"
    );
    assert_eq!(
        fs::read(&plugin).expect("installed plugin"),
        b"pinned-release"
    );
}
