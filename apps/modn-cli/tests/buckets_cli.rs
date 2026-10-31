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
