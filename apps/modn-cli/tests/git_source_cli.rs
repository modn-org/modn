#[path = "support/catalog.rs"]
mod support;

#[path = "support/main_bucket.rs"]
mod main_bucket;

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use support::{error, Fixture};

struct Repository {
    base: PathBuf,
    commit: String,
}

fn git(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git output")
        .trim()
        .to_owned()
}

fn commit_all(directory: &Path, message: &str) {
    git(directory, &["add", "."]);
    git(
        directory,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-m",
            message,
        ],
    );
}

fn repository(root: &Path) -> Repository {
    let work = root.join("git-source-work");
    let plugin = work.join("Demo/package/plugins/git-source.plugin");
    fs::create_dir_all(plugin.parent().expect("plugin parent")).expect("work tree");
    fs::write(&plugin, b"pinned-release").expect("pinned payload");
    git(&work, &["init", "--initial-branch=main"]);
    commit_all(&work, "pinned");
    let commit = git(&work, &["rev-parse", "HEAD"]);
    fs::write(&plugin, b"later-release").expect("later payload");
    commit_all(&work, "later");
    let base = root.join("repos");
    fs::create_dir_all(base.join("fixture")).expect("remote base");
    git(
        root,
        &[
            "clone",
            "--bare",
            "--no-local",
            "--",
            work.to_str().expect("work path"),
            base.join("fixture/project.git")
                .to_str()
                .expect("bare path"),
        ],
    );
    Repository { base, commit }
}

fn file_url(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    }
}

fn add_bucket(fixture: &Fixture, root: &Path, commit: &str) {
    let bucket = root.join("git-fixture");
    let game = bucket.join("mods/test-game");
    fs::create_dir_all(&game).expect("bucket directory");
    fs::write(
        bucket.join("bucket.json"),
        br#"{"schema_version":2,"name":"git-fixture","title":"Git fixture"}"#,
    )
    .expect("bucket metadata");
    let manifest = json!({
        "schema_version": 2,
        "id": "git-package",
        "game": "test-game",
        "name": "Git Package",
        "version": "1.0",
        "sources": [{"kind": "git", "repository": "fixture/project", "commit": commit}],
    });
    fs::write(
        game.join("git-package.json"),
        serde_json::to_vec(&manifest).expect("manifest JSON"),
    )
    .expect("manifest");
    let result = fixture.run_paths(&["bucket", "add", "git-fixture"], &[&bucket]);
    assert!(result.status.success(), "{}", error(&result));
}

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

fn run(fixture: &Fixture, base: &Path, args: &[&str]) -> Output {
    main_bucket::command(env!("CARGO_BIN_EXE_modn"))
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .env("MODN_TEST_GIT_REMOTE_BASE", file_url(base))
        .args(args)
        .output()
        .expect("run CLI")
}

fn installed_plugin(fixture: &Fixture) -> PathBuf {
    fixture.game.join("addons/git-source.plugin")
}

fn cache_archives(fixture: &Fixture) -> Vec<PathBuf> {
    let cache = fixture.state.join("cache");
    match fs::read_dir(cache) {
        Ok(entries) => entries
            .map(|entry| entry.expect("cache entry").path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "zip"))
            .collect(),
        Err(_) => Vec::new(),
    }
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
