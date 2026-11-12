#[path = "support/main_bucket.rs"]
mod main_bucket;

#[path = "support/catalog.rs"]
mod support;

#[path = "support/tree.rs"]
#[allow(dead_code)]
mod tree;

use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use support::{error, Fixture};

const REFERENCE: &str = "git-fixture/test-game/git-package";

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

/// Build a repository with enough entries that packing the pinned tree does
/// not finish before the test observes the download phase and cancels.
fn repository(root: &Path) -> Repository {
    let work = root.join("git-source-work");
    let plugin = work.join("Demo/package/plugins/git-source.plugin");
    fs::create_dir_all(plugin.parent().expect("plugin parent")).expect("work tree");
    fs::write(&plugin, b"pinned-release").expect("pinned payload");
    // Incompressible filler makes packing the pinned tree take long enough
    // for the test to observe the download phase and send its cancel.
    let filler = work.join("Demo/package/plugins/filler.plugin");
    let mut data = Vec::with_capacity(8 * 1024 * 1024);
    let mut state = 0x1234_5678u32;
    for _ in 0..8 * 1024 * 1024 {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        data.push((state >> 24) as u8);
    }
    fs::write(&filler, data).expect("filler payload");
    git(&work, &["init", "--initial-branch=main"]);
    commit_all(&work, "pinned");
    let commit = git(&work, &["rev-parse", "HEAD"]);
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
    let game = bucket.join("games/test-game");
    fs::create_dir_all(&game).expect("bucket directory");
    fs::write(
        bucket.join("bucket.json"),
        br#"{"schema_version":1,"name":"git-fixture","title":"Git fixture"}"#,
    )
    .expect("bucket metadata");
    let manifest = serde_json::json!({
        "schema_version": 1,
        "id": "git-package",
        "game": "test-game",
        "name": "Git Package",
        "version": "1.0",
        "source": {"kind": "git", "repository": "fixture/project", "commit": commit},
    });
    fs::write(
        game.join("git-package.json"),
        serde_json::to_vec(&manifest).expect("manifest JSON"),
    )
    .expect("manifest");
    let result = fixture.run_paths(&["bucket", "add", "git-fixture"], &[&bucket]);
    assert!(result.status.success(), "{}", error(&result));
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

fn cache_archives(fixture: &Fixture) -> Vec<PathBuf> {
    match fs::read_dir(fixture.state.join("cache")) {
        Ok(entries) => entries
            .map(|entry| entry.expect("cache entry").path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "zip"))
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn reports_download(line: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return false;
    };
    value["phase"] == "download" && value["package"] == REFERENCE
}

#[test]
fn progress_json_reports_a_download_phase_for_a_git_source() {
    let fixture = Fixture::new();
    let repository = repository(fixture.temp.path());
    add_bucket(&fixture, fixture.temp.path(), &repository.commit);
    fixture.register();
    let game = fixture.game.to_str().expect("game path");

    let installed = run(
        &fixture,
        &repository.base,
        &[
            "--json",
            "--progress",
            "json",
            "install",
            "test-game/git-package",
            "--path",
            game,
            "--yes",
        ],
    );
    assert!(installed.status.success(), "{}", error(&installed));
    let stderr = String::from_utf8_lossy(&installed.stderr);
    assert!(
        stderr.lines().any(reports_download),
        "no download phase for the Git package: {stderr}"
    );
    assert!(fixture.game.join("addons/git-source.plugin").is_file());
}

#[test]
fn a_cancel_during_a_git_fetch_stops_before_commit_without_cache_or_game_writes() {
    let fixture = Fixture::new();
    let repository = repository(fixture.temp.path());
    add_bucket(&fixture, fixture.temp.path(), &repository.commit);
    fixture.register();
    let game = fixture.game.to_str().expect("game path").to_owned();
    let state_db = fixture.state.join("state.sqlite");
    let state_before = fs::read(&state_db).expect("state bytes");
    let game_before = tree::snapshot(&fixture.game);

    let mut command = main_bucket::command(env!("CARGO_BIN_EXE_modn"));
    command
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .env("MODN_TEST_GIT_REMOTE_BASE", file_url(&repository.base))
        .args([
            "--json",
            "--progress",
            "json",
            "install",
            "test-game/git-package",
            "--path",
            &game,
            "--yes",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn modn");
    let mut stdin = child.stdin.take().expect("stdin");
    let stderr = child.stderr.take().expect("stderr");
    let mut lines = Vec::new();
    let mut cancelled_during_download = false;
    for line in BufReader::new(stderr).lines() {
        let line = line.expect("stderr line");
        if !cancelled_during_download && reports_download(&line) {
            stdin.write_all(b"cancel\n").expect("cancel line");
            stdin.flush().expect("flush cancel");
            cancelled_during_download = true;
        }
        lines.push(line);
    }
    drop(stdin);
    let output = child.wait_with_output().expect("modn output");

    assert!(
        cancelled_during_download,
        "install finished without a download phase: {}",
        error(&output)
    );
    assert!(!output.status.success(), "{}", error(&output));
    let stderr = lines.join("\n");
    assert!(stderr.contains(r#""error":"CANCELLED""#), "{stderr}");
    assert!(
        stderr
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .any(|value| value["phase"] == "result" && value["outcome"] == "cancelled"),
        "{stderr}"
    );
    assert_eq!(tree::snapshot(&fixture.game), game_before);
    assert_eq!(fs::read(&state_db).expect("state bytes"), state_before);
    assert!(
        cache_archives(&fixture).is_empty(),
        "a cancelled Git fetch kept a cache entry"
    );
}
