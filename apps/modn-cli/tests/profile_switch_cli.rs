//! CLI contract for `modn profile switch`.
//!
//! Every test uses a disposable game tree and manager state. Installs copy
//! their archive into the fixture cache so a later switch can restore them.

#[path = "support/profile_fixture.rs"]
mod profile_fixture;

use profile_fixture::ProfileFixture;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};

const ALPHA: &str = "archive/pc/mod/alpha.archive";
const BETA: &str = "archive/pc/mod/beta.archive";
const USER: &str = "archive/pc/mod/user.cfg";

fn package_id(report: &Value) -> String {
    report["id"].as_str().expect("package id").to_owned()
}

fn install(fixture: &ProfileFixture, name: &str) -> String {
    let report = fixture.install(name);
    let root = fixture.state().parent().expect("fixture root");
    let cache = fixture.state().join("cache");
    fs::create_dir_all(&cache).expect("cache directory");
    fs::copy(
        root.join("archives").join(format!("{name}.zip")),
        cache.join(format!("cyberpunk-2077#{name}#1.0#0000000.zip")),
    )
    .expect("cache entry");
    package_id(&report)
}

fn create_second(fixture: &ProfileFixture) {
    fixture.json(&["profile", "create", "second"]);
}

/// Install `beta` into an inactive `second` profile and return to `default`.
fn second_with_beta(fixture: &ProfileFixture, alpha: &str) -> String {
    create_second(fixture);
    fixture.json(&["profile", "switch", "second", "--yes"]);
    let beta = install(fixture, "beta");
    assert_ne!(beta, alpha);
    fixture.json(&["profile", "switch", "default", "--yes"]);
    beta
}

fn progress_error_code(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|value| value["error"].as_str().map(str::to_owned))
        .next_back()
        .expect("error code")
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    let mut entries = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).expect("game directory") {
            let path = entry.expect("entry").path();
            let relative = path
                .strip_prefix(root)
                .expect("relative path")
                .to_path_buf();
            if fs::symlink_metadata(&path).expect("metadata").is_dir() {
                pending.push(path);
                entries.insert(relative, None);
            } else {
                entries.insert(relative, Some(fs::read(&path).expect("file bytes")));
            }
        }
    }
    entries
}

fn active_profile(fixture: &ProfileFixture) -> String {
    let list = fixture.profile_list();
    list["profiles"]
        .as_array()
        .expect("profiles")
        .iter()
        .find(|profile| profile["active"] == true)
        .expect("active profile")["name"]
        .as_str()
        .expect("name")
        .to_owned()
}

#[test]
fn switch_to_an_empty_profile_removes_its_files_and_keeps_unmanaged_files() {
    let fixture = ProfileFixture::new();
    let alpha = install(&fixture, "alpha");
    create_second(&fixture);
    fs::write(fixture.game().join(USER), b"user").expect("unmanaged file");

    let switched = fixture.json(&["profile", "switch", "second", "--yes"]);

    assert_eq!(switched["from"], "default");
    assert_eq!(switched["to"], "second");
    assert_eq!(switched["dry_run"], false);
    assert_eq!(switched["removed"], 1);
    assert_eq!(switched["written"], 0);
    assert_eq!(switched["unchanged"], 0);
    assert_eq!(switched["entering"], json!([]));
    assert_eq!(switched["leaving"], json!([alpha]));
    assert!(switched["operation_id"]
        .as_str()
        .expect("operation id")
        .starts_with("txn-"));
    assert!(!fixture.game().join(ALPHA).exists());
    assert_eq!(fs::read(fixture.game().join(USER)).expect("user"), b"user");
    assert_eq!(active_profile(&fixture), "second");
}

#[test]
fn switch_back_restores_the_original_tree_byte_identical() {
    let fixture = ProfileFixture::new();
    let alpha = install(&fixture, "alpha");
    let _beta = second_with_beta(&fixture, &alpha);
    let initial = snapshot(fixture.game());
    let installed = fixture.json(&["list"]);

    fixture.json(&["profile", "switch", "second", "--yes"]);
    assert_eq!(fs::read(fixture.game().join(BETA)).expect("beta"), b"beta");
    assert!(!fixture.game().join(ALPHA).exists());
    fixture.json(&["profile", "switch", "default", "--yes"]);

    assert_eq!(
        fs::read(fixture.game().join(ALPHA)).expect("alpha"),
        b"alpha"
    );
    assert!(!fixture.game().join(BETA).exists());
    assert_eq!(snapshot(fixture.game()), initial);
    assert_eq!(fixture.json(&["list"]), installed);
    assert_eq!(active_profile(&fixture), "default");
}

#[test]
fn dry_run_human_and_json_list_the_paths_and_change_nothing() {
    let fixture = ProfileFixture::new();
    let alpha = install(&fixture, "alpha");
    let beta = second_with_beta(&fixture, &alpha);
    let before_tree = snapshot(fixture.game());
    let before_list = fixture.json(&["list"]);
    let before_profiles = fixture.profile_list();

    let human = fixture.run(&["profile", "switch", "second", "--dry-run"]);
    assert!(
        human.status.success(),
        "{}",
        String::from_utf8_lossy(&human.stderr)
    );
    let text = String::from_utf8(human.stdout).expect("UTF-8 output");
    assert!(text.contains("Switching profile"), "{text}");
    assert!(text.contains("to remove"), "{text}");
    assert!(text.contains("to write"), "{text}");
    assert!(text.contains("unchanged"), "{text}");
    assert!(text.contains(ALPHA), "{text}");
    assert!(text.contains(BETA), "{text}");

    let report = fixture.json(&["profile", "switch", "second", "--dry-run"]);
    assert_eq!(report["dry_run"], true);
    assert_eq!(report["from"], "default");
    assert_eq!(report["to"], "second");
    assert_eq!(report["remove"][0]["path"], ALPHA);
    assert_eq!(report["remove"][0]["action"], "remove");
    assert_eq!(report["remove"][0]["package"], alpha);
    assert_eq!(report["write"][0]["path"], BETA);
    assert_eq!(report["write"][0]["action"], "write");
    assert_eq!(report["write"][0]["package"], beta);
    assert_eq!(report["unchanged"], 0);
    assert_eq!(report["bytes_to_write"], 4);
    assert_eq!(report["entering"], json!([beta]));
    assert_eq!(report["leaving"], json!([alpha]));

    assert_eq!(snapshot(fixture.game()), before_tree);
    assert_eq!(fixture.json(&["list"]), before_list);
    assert_eq!(fixture.profile_list(), before_profiles);
}

#[test]
fn a_real_run_without_yes_is_refused_and_changes_nothing() {
    let fixture = ProfileFixture::new();
    install(&fixture, "alpha");
    create_second(&fixture);
    let before = snapshot(fixture.game());
    let profiles = fixture.profile_list();

    let (code, message) = fixture.error(&["profile", "switch", "second"]);

    assert_eq!(code, "APPROVAL_REQUIRED");
    assert!(message.contains("--yes"), "{message}");
    assert!(message.contains("--dry-run"), "{message}");
    assert_eq!(snapshot(fixture.game()), before);
    assert_eq!(fixture.profile_list(), profiles);
}

#[test]
fn switch_to_the_active_profile_is_refused() {
    let fixture = ProfileFixture::new();
    install(&fixture, "alpha");
    let before = snapshot(fixture.game());

    let (code, message) = fixture.error(&["profile", "switch", "default", "--yes"]);

    assert_eq!(code, "PROFILE_ACTIVE");
    assert!(message.contains("already active"), "{message}");
    assert_eq!(snapshot(fixture.game()), before);
}

#[test]
fn switch_to_an_unknown_profile_is_refused() {
    let fixture = ProfileFixture::new();
    install(&fixture, "alpha");
    let before = snapshot(fixture.game());

    let (code, message) = fixture.error(&["profile", "switch", "missing", "--yes"]);

    assert_eq!(code, "PROFILE_NOT_FOUND");
    assert!(message.contains("missing"), "{message}");
    assert_eq!(snapshot(fixture.game()), before);
}

#[test]
fn drift_in_a_managed_file_is_refused_and_nothing_changes() {
    let fixture = ProfileFixture::new();
    install(&fixture, "alpha");
    create_second(&fixture);
    fs::write(fixture.game().join(ALPHA), b"edited").expect("drift");
    let before = snapshot(fixture.game());
    let profiles = fixture.profile_list();

    let (code, message) = fixture.error(&["profile", "switch", "second", "--yes"]);

    assert_eq!(code, "PROFILE_DRIFT");
    assert!(message.contains("modn doctor"), "{message}");
    assert_eq!(snapshot(fixture.game()), before);
    assert_eq!(fixture.profile_list(), profiles);
    assert_eq!(active_profile(&fixture), "default");
}

#[test]
fn a_missing_cache_archive_is_refused_and_nothing_changes() {
    let fixture = ProfileFixture::new();
    let alpha = install(&fixture, "alpha");
    second_with_beta(&fixture, &alpha);
    fs::remove_dir_all(fixture.state().join("cache")).expect("remove cache");
    let before = snapshot(fixture.game());
    let profiles = fixture.profile_list();

    let (code, message) = fixture.error(&["profile", "switch", "second", "--yes"]);

    assert_eq!(code, "PROFILE_ARCHIVE_MISSING");
    assert!(message.contains("modn cache show"), "{message}");
    assert_eq!(snapshot(fixture.game()), before);
    assert_eq!(fixture.profile_list(), profiles);
    assert_eq!(active_profile(&fixture), "default");
}

#[test]
fn a_pending_transaction_is_refused() {
    let fixture = ProfileFixture::new();
    install(&fixture, "alpha");
    create_second(&fixture);
    let connection =
        rusqlite::Connection::open(fixture.state().join("state.sqlite")).expect("state database");
    connection
        .execute(
            "INSERT INTO pending_transactions
             (id, action, status, game_path, package_id, archive_blake3)
             VALUES ('txn-pending', 'install', 'pending', ?1, '', NULL)",
            [fixture.game().to_string_lossy().as_ref()],
        )
        .expect("pending transaction");
    drop(connection);
    let before = snapshot(fixture.game());

    let (code, message) = fixture.error(&["profile", "switch", "second", "--yes"]);

    assert_eq!(code, "PENDING_TRANSACTION");
    assert!(message.contains("modn recover"), "{message}");
    assert_eq!(snapshot(fixture.game()), before);
    assert_eq!(active_profile(&fixture), "default");
}

#[test]
fn progress_json_reports_every_switch_phase_on_stderr() {
    let fixture = ProfileFixture::new();
    let alpha = install(&fixture, "alpha");
    second_with_beta(&fixture, &alpha);

    let output = fixture
        .command()
        .args([
            "--json",
            "--progress",
            "json",
            "profile",
            "switch",
            "second",
            "--yes",
        ])
        .output()
        .expect("run the modn binary");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("stdout JSON");
    assert_eq!(report["to"], "second");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let names: Vec<String> = stderr
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|value| value["phase"].as_str().map(str::to_owned))
        .collect();
    assert_eq!(
        names,
        ["resolve", "extract", "stage", "commit", "result"],
        "{stderr}"
    );
    assert!(stderr.contains(r#""outcome":"succeeded""#), "{stderr}");
}

#[test]
fn progress_json_without_yes_is_refused_before_any_change() {
    let fixture = ProfileFixture::new();
    install(&fixture, "alpha");
    create_second(&fixture);
    let before = snapshot(fixture.game());

    let output = fixture
        .command()
        .args([
            "--json",
            "--progress",
            "json",
            "profile",
            "switch",
            "second",
        ])
        .output()
        .expect("run the modn binary");

    assert!(!output.status.success());
    assert_eq!(progress_error_code(&output), "APPROVAL_REQUIRED");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("pass --yes with --progress json"),
        "{stderr}"
    );
    assert_eq!(snapshot(fixture.game()), before);
    assert_eq!(active_profile(&fixture), "default");
}

#[test]
fn a_cancel_line_stops_the_switch_before_the_commit_point() {
    let fixture = ProfileFixture::new();
    let alpha = install(&fixture, "alpha");
    second_with_beta(&fixture, &alpha);
    let before = snapshot(fixture.game());

    let mut command = fixture.command();
    command
        .args([
            "--json",
            "--progress",
            "json",
            "profile",
            "switch",
            "second",
            "--yes",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn modn");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"cancel\n")
        .expect("cancel line");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("modn output");

    assert!(!output.status.success());
    assert_eq!(progress_error_code(&output), "CANCELLED");
    assert_eq!(snapshot(fixture.game()), before);
    assert_eq!(active_profile(&fixture), "default");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(r#""outcome":"cancelled""#), "{stderr}");
}

#[test]
fn recover_rolls_back_an_interrupted_switch() {
    let fixture = ProfileFixture::new();
    let alpha = install(&fixture, "alpha");
    let _beta = second_with_beta(&fixture, &alpha);
    let before = snapshot(fixture.game());
    let id = "txn-switch-crashed";
    let destination = fixture.game().join(ALPHA);
    let witness = destination
        .parent()
        .expect("destination parent")
        .join(format!(".modn-txn-{id}-1.old"));
    fs::rename(&destination, &witness).expect("quarantine witness");
    let connection =
        rusqlite::Connection::open(fixture.state().join("state.sqlite")).expect("state database");
    connection
        .execute(
            "INSERT INTO pending_transactions
             (id, action, status, game_path, package_id, archive_blake3)
             VALUES (?1, 'profile-switch', 'pending', ?2, '', NULL)",
            rusqlite::params![id, fixture.game().to_string_lossy().as_ref()],
        )
        .expect("pending transaction");
    connection
        .execute(
            "INSERT INTO pending_profile_switches(transaction_id, game_path, from_profile, to_profile)
             VALUES (?1, ?2, 'default', 'second')",
            rusqlite::params![id, fixture.game().to_string_lossy().as_ref()],
        )
        .expect("pending switch");
    connection
        .execute(
            "INSERT INTO pending_files(transaction_id, ord, root, relative_path, blake3, bytes)
             VALUES (?1, 0, 'game', ?2, ?3, 4)",
            rusqlite::params![id, BETA, digest(b"beta")],
        )
        .expect("pending write");
    connection
        .execute(
            "INSERT INTO pending_overrides
             (transaction_id, ord, root, relative_path, package_id, blake3, bytes)
             VALUES (?1, 1, 'game', ?2, ?3, ?4, 5)",
            rusqlite::params![id, ALPHA, alpha, digest(b"alpha")],
        )
        .expect("pending removal");
    drop(connection);
    assert_ne!(snapshot(fixture.game()), before);

    let recovered = fixture.json(&["recover", "--yes"]);

    assert_eq!(recovered["recovered"], 1);
    assert_eq!(snapshot(fixture.game()), before);
    assert_eq!(active_profile(&fixture), "default");
    assert!(!witness.exists());
}

#[test]
fn undo_of_a_switch_is_refused() {
    let fixture = ProfileFixture::new();
    install(&fixture, "alpha");
    create_second(&fixture);
    let switched = fixture.json(&["profile", "switch", "second", "--yes"]);
    let operation = switched["operation_id"]
        .as_str()
        .expect("operation id")
        .to_owned();
    let before = snapshot(fixture.game());

    let (code, message) = fixture.error(&["undo", &operation, "--yes"]);

    assert_eq!(code, "UNDO_CONFLICT");
    assert!(
        message.contains("reversing a profile switch is a new switch"),
        "{message}"
    );
    assert_eq!(snapshot(fixture.game()), before);
    assert_eq!(active_profile(&fixture), "second");
}

#[test]
fn the_active_profile_shows_in_profile_list_json() {
    let fixture = ProfileFixture::new();
    install(&fixture, "alpha");
    create_second(&fixture);

    let human = fixture.run(&["profile", "switch", "second", "--yes"]);
    assert!(
        human.status.success(),
        "{}",
        String::from_utf8_lossy(&human.stderr)
    );
    let text = String::from_utf8(human.stdout).expect("UTF-8 output");
    assert!(
        text.contains("Switching profile \"default\" -> \"second\""),
        "{text}"
    );
    assert!(text.contains("Switched to profile \"second\""), "{text}");
    let list = fixture.profile_list();
    let profiles = list["profiles"].as_array().expect("profiles");
    let active: Vec<&str> = profiles
        .iter()
        .filter(|profile| profile["active"] == true)
        .map(|profile| profile["name"].as_str().expect("name"))
        .collect();
    assert_eq!(active, ["second"]);

    fixture.json(&["profile", "switch", "default", "--yes"]);
    assert_eq!(active_profile(&fixture), "default");
}

fn digest(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}
