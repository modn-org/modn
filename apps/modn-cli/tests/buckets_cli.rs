use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

fn run(state: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_modn"))
        .args([
            "--state-dir",
            state.to_str().expect("test fixture invariant"),
            "--json",
            "bucket",
        ])
        .args(args)
        .output()
        .expect("test fixture invariant")
}

fn seed(root: &Path, body: &str) {
    let directory = root.join("games/test-game");
    fs::create_dir_all(&directory).expect("test fixture invariant");
    fs::write(
        root.join("bucket.json"),
        r#"{"schema_version":1,"name":"community","title":"Community"}"#,
    )
    .expect("test fixture invariant");
    fs::write(directory.join("example.json"), body).expect("test fixture invariant");
}

fn valid_manifest() -> String {
    serde_json::json!({
        "schema_version":1,"id":"example","game":"test-game","name":"Example",
        "version":"1.0","archive_format":"zip",
        "sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "source":{"kind":"manual","filename":"example.zip"},"dependencies":[]
    })
    .to_string()
}

fn output(result: &Output) -> String {
    String::from_utf8_lossy(&result.stderr).to_string()
}

#[test]
fn bundled_main_is_available_without_mutating_state() {
    let temp = TempDir::new().expect("test fixture invariant");
    let result = run(temp.path(), &["list"]);
    assert!(result.status.success(), "{}", output(&result));
    let entries: serde_json::Value =
        serde_json::from_slice(&result.stdout).expect("test fixture invariant");
    assert_eq!(entries.as_array().expect("test fixture invariant").len(), 1);
    assert_eq!(entries[0]["name"], "main");
    assert!(!temp.path().join("buckets").exists());
    assert!(!temp.path().join("state.sqlite").exists());
    let result = run(temp.path(), &["show", "main/cyberpunk-2077/red4ext"]);
    assert!(result.status.success(), "{}", output(&result));
    let entry: serde_json::Value =
        serde_json::from_slice(&result.stdout).expect("test fixture invariant");
    assert_eq!(entry["version"], "1.31.0");
    assert_eq!(entry["source"]["kind"], "manual");
    assert!(!temp.path().join("state.sqlite").exists());
}

#[test]
fn local_registration_resolves_and_rejects_duplicate_registration() {
    let temp = TempDir::new().expect("test fixture invariant");
    let bucket = temp.path().join("source");
    seed(&bucket, &valid_manifest());
    let state = temp.path().join("state");
    let add = run(
        &state,
        &[
            "add",
            "community",
            bucket.to_str().expect("test fixture invariant"),
        ],
    );
    assert!(add.status.success(), "{}", output(&add));
    let search = run(&state, &["search"]);
    assert!(search.status.success(), "{}", output(&search));
    let entries: serde_json::Value =
        serde_json::from_slice(&search.stdout).expect("test fixture invariant");
    assert_eq!(entries.as_array().expect("test fixture invariant").len(), 4);
    let show = run(&state, &["show", "community/test-game/example"]);
    assert!(show.status.success(), "{}", output(&show));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&show.stdout).expect("test fixture invariant")
            ["id"],
        "example"
    );
    assert!(!run(
        &state,
        &[
            "add",
            "community",
            bucket.to_str().expect("test fixture invariant")
        ]
    )
    .status
    .success());
    let removed = run(&state, &["remove", "community"]);
    assert!(removed.status.success(), "{}", output(&removed));
    assert!(bucket.join("bucket.json").exists());
    let search = run(&state, &["search"]);
    assert!(search.status.success(), "{}", output(&search));
    let after: serde_json::Value =
        serde_json::from_slice(&search.stdout).expect("test fixture invariant");
    assert_eq!(after.as_array().expect("array").len(), 3);
    assert!(!run(&state, &["remove", "main"]).status.success());
    assert!(!state.join("state.sqlite").exists());
}

#[test]
fn malformed_packages_are_rejected_before_registration() {
    let temp = TempDir::new().expect("test fixture invariant");
    let bucket = temp.path().join("source");
    let state = temp.path().join("state");
    let mut manifest: serde_json::Value =
        serde_json::from_str(&valid_manifest()).expect("test fixture invariant");
    for (field, value) in [
        ("sha256", serde_json::json!("bad")),
        ("id", serde_json::json!("../escape")),
        ("dependencies", serde_json::json!(["example"])),
        (
            "source",
            serde_json::json!({"kind":"manual","filename":"../bad.zip"}),
        ),
        ("schema_version", serde_json::json!(2)),
    ] {
        let original = manifest[field].clone();
        manifest[field] = value;
        seed(&bucket, &manifest.to_string());
        let result = run(
            &state,
            &[
                "add",
                "community",
                bucket.to_str().expect("test fixture invariant"),
            ],
        );
        assert!(!result.status.success(), "{field}");
        assert!(!state.join("buckets").exists(), "{field}");
        manifest[field] = original;
    }
}

#[test]
fn registered_bucket_corruption_never_silently_falls_back() {
    let temp = TempDir::new().expect("test fixture invariant");
    let bucket = temp.path().join("source");
    let state = temp.path().join("state");
    seed(&bucket, &valid_manifest());
    assert!(run(
        &state,
        &[
            "add",
            "community",
            bucket.to_str().expect("test fixture invariant")
        ]
    )
    .status
    .success());
    fs::write(bucket.join("games/test-game/example.json"), b"{bad")
        .expect("test fixture invariant");
    let failed = run(&state, &["search"]);
    assert!(!failed.status.success());
    assert!(output(&failed).contains("invalid bucket JSON"));
    assert!(!state.join("state.sqlite").exists());
}

#[cfg(unix)]
#[test]
fn linked_manifest_is_refused() {
    use std::os::unix::fs::symlink;
    let temp = TempDir::new().expect("test fixture invariant");
    let bucket = temp.path().join("source");
    seed(&bucket, &valid_manifest());
    let manifest = bucket.join("games/test-game/example.json");
    let other = temp.path().join("outside.json");
    fs::rename(&manifest, &other).expect("test fixture invariant");
    symlink(&other, &manifest).expect("test fixture invariant");
    assert!(!run(
        &temp.path().join("state"),
        &[
            "add",
            "community",
            bucket.to_str().expect("test fixture invariant")
        ]
    )
    .status
    .success());
}

#[test]
fn leftover_temp_files_outside_registry_do_not_block_reads() {
    let temp = TempDir::new().expect("test fixture invariant");
    let bucket = temp.path().join("source");
    let state = temp.path().join("state");
    seed(&bucket, &valid_manifest());
    assert!(run(
        &state,
        &["add", "community", bucket.to_str().expect("path")]
    )
    .status
    .success());
    let leftover = state.join("buckets").join(".orphaned-staging");
    fs::write(&leftover, b"incomplete registration").expect("orphan");
    let found = run(&state, &["search"]);
    assert!(found.status.success(), "{}", output(&found));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&found.stdout)
            .expect("JSON")
            .as_array()
            .expect("array")
            .len(),
        4
    );
    assert!(run(&state, &["remove", "community"]).status.success());
    assert!(
        leftover.exists(),
        "registry removal must not claim stray files"
    );
}

#[test]
fn linked_bucket_registry_parent_cannot_redirect_registration_writes() {
    let temp = TempDir::new().expect("test fixture invariant");
    let source = temp.path().join("source");
    seed(&source, &valid_manifest());
    let state = temp.path().join("state");
    let outside = temp.path().join("external-target");
    fs::create_dir_all(&state).expect("state");
    fs::create_dir_all(&outside).expect("outside");
    let link = state.join("buckets");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &link).expect("link");
    #[cfg(windows)]
    {
        let result = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&outside)
            .output()
            .expect("create junction");
        assert!(result.status.success(), "{}", output(&result));
    }
    let result = run(
        &state,
        &["add", "community", source.to_str().expect("path")],
    );
    assert!(
        !result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    assert!(fs::read_dir(&outside).expect("outside").next().is_none());
}

#[cfg(unix)]
#[test]
fn named_pipe_metadata_is_rejected_without_opening_it() {
    let temp = TempDir::new().expect("test fixture invariant");
    let bucket = temp.path().join("source");
    seed(&bucket, &valid_manifest());
    let path = bucket.join("games/test-game/example.json");
    fs::remove_file(&path).expect("remove fixture");
    let creation = Command::new("mkfifo").arg(&path).output().expect("mkfifo");
    assert!(
        creation.status.success(),
        "{}",
        String::from_utf8_lossy(&creation.stderr)
    );
    let result = run(
        &temp.path().join("state"),
        &["add", "community", bucket.to_str().expect("path")],
    );
    assert!(!result.status.success());
    assert!(output(&result).contains("regular file"));
}

#[test]
fn bucket_registration_does_not_write_into_detected_game_root() {
    let temp = TempDir::new().expect("test fixture invariant");
    let bucket = temp.path().join("source");
    seed(&bucket, &valid_manifest());
    let game = temp.path().join("game");
    let marker = game.join("bin/x64/Cyberpunk2077.exe");
    fs::create_dir_all(marker.parent().expect("marker parent")).expect("game");
    fs::write(&marker, b"unmodified game marker").expect("marker");
    // No explicit --game: the built-in bucket metadata supplies the game ID.
    let result = run(&game, &["add", "community", bucket.to_str().expect("path")]);
    assert!(
        !result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    assert!(
        output(&result).contains("outside a game installation"),
        "{}",
        output(&result)
    );
    assert!(!game.join("buckets").exists());
    assert_eq!(
        fs::read(&marker).expect("marker"),
        b"unmodified game marker"
    );
}

#[test]
fn bucket_title_control_characters_are_rejected() {
    let temp = TempDir::new().expect("test fixture invariant");
    let bucket = temp.path().join("source");
    seed(&bucket, &valid_manifest());
    fs::write(
        bucket.join("bucket.json"),
        r#"{"schema_version":1,"name":"community","title":"Community\u001b[2J"}"#,
    )
    .expect("metadata");
    let state = temp.path().join("state");
    let result = run(
        &state,
        &["add", "community", bucket.to_str().expect("path")],
    );
    assert!(
        !result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    assert!(!state.join("buckets").exists());
}

#[test]
fn bucket_registry_destination_inside_game_root_is_refused() {
    let temp = TempDir::new().expect("test fixture invariant");
    let source = temp.path().join("source");
    seed(&source, &valid_manifest());
    let state = temp.path().join("state");
    let game = state.join("buckets");
    let marker = game.join("bin/x64/Cyberpunk2077.exe");
    fs::create_dir_all(marker.parent().expect("marker parent")).expect("fixture game");
    fs::write(&marker, b"original marker").expect("fixture game marker");
    let result = run(
        &state,
        &["add", "community", source.to_str().expect("source path")],
    );
    assert!(!result.status.success(), "{}", output(&result));
    assert!(
        output(&result).contains("outside a game installation"),
        "{}",
        output(&result)
    );
    assert!(!game.join("registrations").exists());
    assert_eq!(fs::read(&marker).expect("marker"), b"original marker");
    assert!(!state.join("state.sqlite").exists());
}

#[test]
fn obsolete_module_directory_is_not_a_valid_bucket() {
    let temp = TempDir::new().expect("fixture");
    let bucket = temp.path().join("source");
    let state = temp.path().join("state");
    seed(&bucket, &valid_manifest());
    fs::create_dir_all(bucket.join("modules/test-game")).expect("fixture");
    let result = run(
        &state,
        &["add", "community", bucket.to_str().expect("path")],
    );
    assert!(!result.status.success());
    assert!(output(&result).contains("bucket module directories are unsupported"));
    assert!(!state.join("buckets").exists());
    assert!(!run(&state, &["modules"]).status.success());
}

#[test]
fn package_catalog_remains_independent_of_game_module_source() {
    let temp = TempDir::new().expect("fixture");
    let state = temp.path().join("state");
    let result = run(&state, &["show", "main/cyberpunk-2077/cet"]);
    assert!(result.status.success(), "{}", output(&result));
    let entry: serde_json::Value = serde_json::from_slice(&result.stdout).expect("json");
    assert_eq!(entry["id"], "cet");
    assert!(!state.exists());
}

#[test]
fn invalid_development_source_cannot_redirect_bucket_registry_into_game() {
    let temp = TempDir::new().expect("fixture");
    let bucket = temp.path().join("community");
    seed(&bucket, &valid_manifest());
    let game = temp.path().join("game");
    let marker = game.join("bin/x64/Cyberpunk2077.exe");
    fs::create_dir_all(marker.parent().expect("parent")).expect("fixture");
    fs::write(&marker, b"unchanged").expect("marker");
    let invalid = temp.path().join("missing-module");
    let mismatched = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/test-game");

    let broken = temp.path().join("broken-policy");
    fs::create_dir(&broken).expect("fixture");
    fs::write(
        broken.join("module.json"),
        r#"{"game_id":"cyberpunk-2077","version":"1.0","api_version":1,"entrypoint":"main.lua"}"#,
    )
    .expect("manifest");
    fs::write(
        broken.join("main.lua"),
        "return { game_id='cyberpunk-2077', api_version=1, validate=function() error('broken validation') end }",
    )
    .expect("Lua");
    for source in [&invalid, &mismatched, &broken] {
        let result = Command::new(env!("CARGO_BIN_EXE_modn"))
            .args(["--state-dir"])
            .arg(&game)
            .arg("--src")
            .arg(source)
            .args(["bucket", "add", "community"])
            .arg(&bucket)
            .output()
            .expect("invoke bucket add");
        assert!(
            !result.status.success(),
            "must reject source: {}",
            source.display()
        );
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("module")
                || String::from_utf8_lossy(&result.stderr).contains("broken validation"),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!game.join("buckets").exists());
        assert!(!game.join("state.sqlite").exists());
        assert_eq!(fs::read(&marker).expect("marker"), b"unchanged");
    }
    // A valid first-party policy still detects this game and denies the write.
    let normal = run(
        &game,
        &["add", "community", bucket.to_str().expect("bucket")],
    );
    assert!(!normal.status.success());
    assert!(output(&normal).contains("outside a game installation"));
    assert!(!game.join("buckets").exists());
}
