use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

fn run(state: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_modn"))
        .args([
            "--state-dir",
            state.to_str().expect("fixture"),
            "--json",
            "bucket",
        ])
        .args(args)
        .output()
        .expect("run fixture CLI")
}

fn seed(root: &Path, bucket: &str, game: &str, version: &str, script: &[u8]) {
    fs::create_dir_all(root).expect("fixture");
    fs::write(
        root.join("bucket.json"),
        format!(r#"{{"schema_version":1,"name":"{bucket}","title":"Test Bucket"}}"#),
    )
    .expect("fixture");
    let release = root.join("modules").join(game).join(version);
    fs::create_dir_all(&release).expect("fixture");
    fs::write(release.join("main.lua"), script).expect("fixture");
    fs::write(
        release.join("module.json"),
        serde_json::json!({
            "schema_version":1,"game_id":game,"version":version,
            "api_version":1,"entrypoint":"main.lua",
            "content_sha256":format!("{:x}", Sha256::digest(script))
        })
        .to_string(),
    )
    .expect("fixture");
}

fn add(state: &Path, root: &Path, name: &str) -> Output {
    run(state, &["add", name, root.to_str().expect("fixture")])
}

fn stderr(result: &Output) -> String {
    String::from_utf8_lossy(&result.stderr).into_owned()
}

#[test]
fn official_module_is_available_offline_without_state_or_execution() {
    let temp = TempDir::new().expect("fixture");
    let state = temp.path().join("state");
    let list = run(&state, &["modules"]);
    assert!(list.status.success(), "{}", stderr(&list));
    let entries: serde_json::Value = serde_json::from_slice(&list.stdout).expect("JSON");
    assert_eq!(entries.as_array().expect("array").len(), 1);
    assert_eq!(entries[0]["bucket"], "main");
    assert_eq!(entries[0]["game_id"], "cyberpunk-2077");
    assert_eq!(entries[0]["api_version"], 1);
    let show = run(&state, &["module-show", "main/cyberpunk-2077/0.1.0"]);
    assert!(show.status.success(), "{}", stderr(&show));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&show.stdout).expect("JSON"),
        entries[0]
    );
    assert!(!state.exists(), "metadata-only reads must not create state");
}

#[test]
fn module_only_buckets_are_namespaced_and_lua_is_never_executed() {
    let temp = TempDir::new().expect("fixture");
    let state = temp.path().join("state");
    for name in ["alpha", "beta"] {
        let root = temp.path().join(name);
        seed(
            &root,
            name,
            "test-game",
            "1.0.0",
            b"error('catalog must not execute Lua')",
        );
        let result = add(&state, &root, name);
        assert!(result.status.success(), "{name}: {}", stderr(&result));
    }
    let found = run(&state, &["modules"]);
    assert!(found.status.success(), "{}", stderr(&found));
    let entries: serde_json::Value = serde_json::from_slice(&found.stdout).expect("JSON");
    assert_eq!(entries.as_array().expect("array").len(), 3);
    assert_eq!(entries[0]["bucket"], "alpha");
    assert_eq!(entries[1]["bucket"], "beta");
    assert_eq!(entries[2]["bucket"], "main");
    let shown = run(&state, &["module-show", "beta/test-game/1.0.0"]);
    assert!(shown.status.success(), "{}", stderr(&shown));
    let shown: serde_json::Value = serde_json::from_slice(&shown.stdout).expect("JSON");
    assert_eq!(shown["bucket"], "beta");
    let packages = run(&state, &["search"]);
    assert!(packages.status.success(), "{}", stderr(&packages));
    let entries: serde_json::Value = serde_json::from_slice(&packages.stdout).expect("JSON");
    assert_eq!(
        entries.as_array().expect("array").len(),
        3,
        "package contract preserved"
    );
    assert!(!state.join("state.sqlite").exists());
}

#[test]
fn malformed_module_manifests_and_entries_fail_before_registration() {
    let temp = TempDir::new().expect("fixture");
    let root = temp.path().join("community");
    let state = temp.path().join("state");
    seed(&root, "community", "test-game", "1.0.0", b"return {}");
    let manifest_path = root.join("modules/test-game/1.0.0/module.json");
    let original: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).expect("fixture")).expect("JSON");
    for (field, invalid) in [
        ("schema_version", serde_json::json!(2)),
        ("game_id", serde_json::json!("other-game")),
        ("version", serde_json::json!("2.0.0")),
        ("api_version", serde_json::json!(999)),
        ("entrypoint", serde_json::json!("../escape.lua")),
        ("content_sha256", serde_json::json!("0".repeat(64))),
    ] {
        let mut altered = original.clone();
        altered[field] = invalid;
        fs::write(&manifest_path, altered.to_string()).expect("fixture");
        let result = add(&state, &root, "community");
        assert!(!result.status.success(), "{field}");
        assert!(!state.join("buckets").exists(), "{field}");
    }
    fs::write(&manifest_path, original.to_string()).expect("fixture");
    fs::write(root.join("modules/test-game/1.0.0/extra.lua"), b"return {}").expect("fixture");
    assert!(!add(&state, &root, "community").status.success());
    fs::remove_file(root.join("modules/test-game/1.0.0/extra.lua")).expect("fixture");
    fs::write(root.join("modules/test-game/1.0.0/main.lua"), [0xff, 0xfe]).expect("fixture");
    assert!(!add(&state, &root, "community").status.success());
    assert!(!state.join("buckets").exists());
}

#[test]
fn registered_module_source_changes_fail_closed() {
    let temp = TempDir::new().expect("fixture");
    let root = temp.path().join("community");
    let state = temp.path().join("state");
    seed(&root, "community", "test-game", "1.0.0", b"return {}");
    let result = add(&state, &root, "community");
    assert!(result.status.success(), "{}", stderr(&result));
    fs::write(
        root.join("modules/test-game/1.0.0/main.lua"),
        b"return {changed=true}",
    )
    .expect("fixture");
    let result = run(&state, &["modules"]);
    assert!(!result.status.success());
    assert!(stderr(&result).contains("SHA-256 mismatch"));
    assert!(
        !run(&state, &["search"]).status.success(),
        "catalog reads fail closed"
    );
    assert!(!state.join("state.sqlite").exists());
}

#[cfg(unix)]
#[test]
fn module_directory_alias_and_case_collisions_are_refused() {
    use std::os::unix::fs::symlink;
    let temp = TempDir::new().expect("fixture");
    let root = temp.path().join("community");
    let state = temp.path().join("state");
    seed(&root, "community", "test-game", "V1", b"return {}");
    seed(&root, "community", "test-game", "v1", b"return {}");
    assert!(!add(&state, &root, "community").status.success());
    fs::remove_dir_all(root.join("modules/test-game/v1")).expect("fixture");
    let external = temp.path().join("outside");
    fs::create_dir(&external).expect("fixture");
    symlink(&external, root.join("modules/test-game/v1")).expect("fixture");
    assert!(!add(&state, &root, "community").status.success());
}

#[test]
fn duplicate_package_ids_across_buckets_are_not_ambiguous() {
    let temp = TempDir::new().expect("fixture");
    let state = temp.path().join("state");
    for name in ["alpha", "beta"] {
        let root = temp.path().join(name);
        seed(&root, name, "test-game", "1.0.0", b"return {}");
        let packages = root.join("games/test-game");
        fs::create_dir_all(&packages).expect("fixture");
        fs::write(
            packages.join("example.json"),
            serde_json::json!({
                "schema_version":1,"id":"example","game":"test-game",
                "name":"Same display name","version":"1.0.0","archive_format":"zip",
                "sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "source":{"kind":"manual","filename":"example.zip"}
            })
            .to_string(),
        )
        .expect("fixture");
        assert!(add(&state, &root, name).status.success());
    }
    for name in ["alpha", "beta"] {
        let shown = run(&state, &["show", &format!("{name}/test-game/example")]);
        assert!(shown.status.success(), "{}", stderr(&shown));
        let entry: serde_json::Value = serde_json::from_slice(&shown.stdout).expect("JSON");
        assert_eq!(entry["bucket"], name);
    }
    let entries = run(&state, &["search"]);
    assert!(entries.status.success(), "{}", stderr(&entries));
    let entries: serde_json::Value = serde_json::from_slice(&entries.stdout).expect("JSON");
    assert_eq!(entries.as_array().expect("array").len(), 5);
}
