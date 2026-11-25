#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "support/zip_catalog.rs"]
mod zip_catalog;

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../buckets/main")
}

fn fixture_tree(title: &str) -> tempfile::TempDir {
    let tree = tempfile::tempdir().expect("fixture tree directory");
    zip_catalog::copy_catalog(&repository(), tree.path());
    let metadata = json!({"schema_version": 2, "name": "main", "title": title});
    fs::write(
        tree.path().join("bucket.json"),
        serde_json::to_vec(&metadata).expect("fixture bucket JSON"),
    )
    .expect("fixture bucket metadata");
    tree
}

fn asset(name: &str, path: &Path) -> Value {
    json!({
        "name": name,
        "browser_download_url": path.to_str().expect("fixture UTF-8 path"),
    })
}

fn release(root: &Path, tag: &str, tree: &Path, requires: Option<&str>) -> Value {
    let directory = root.join(tag);
    fs::create_dir_all(&directory).expect("release directory");
    let zip = directory.join("main-bucket.zip");
    zip_catalog::pack(tree, &zip);
    let sidecar = PathBuf::from(format!("{}.minisig", zip.display()));
    main_bucket::write_signature(tree, &zip);
    let signature = directory.join("listed-signature.minisig");
    fs::rename(sidecar, &signature).expect("listed signature asset");
    let mut assets = vec![
        asset("main-bucket.zip", &zip),
        asset("main-bucket.zip.minisig", &signature),
    ];
    if let Some(requires) = requires {
        let path = directory.join("main-bucket.requires.json");
        fs::write(&path, requires).expect("requires asset");
        assets.push(asset("main-bucket.requires.json", &path));
    }
    json!({
        "tag_name": tag,
        "draft": false,
        "prerelease": false,
        "assets": assets,
    })
}

fn release_list(root: &Path, releases: &[Value]) -> PathBuf {
    let path = root.join("releases.json");
    fs::write(
        &path,
        serde_json::to_vec(&Value::Array(releases.to_vec())).expect("release list JSON"),
    )
    .expect("release list");
    path
}

fn run(data: &Path, list: &Path, args: &[&str]) -> Output {
    Command::new(main_bucket::modn_binary())
        .env("LOCALAPPDATA", data)
        .env("APPDATA", data)
        .env("MODN_APPDATA", data)
        .env("MODN_TEST_MAIN_BUCKET_RELEASES", list)
        .env_remove("MODN_MAIN_BUCKET_SOURCE")
        .env(
            "MODN_TEST_MAIN_BUCKET_PUBLIC_KEY",
            main_bucket::test_public_key(),
        )
        .env_remove("MODN_GAME_MODULES_DIR")
        .args(args)
        .output()
        .expect("run fixture command")
}

fn success(output: Output) -> Output {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn installed_title(data: &Path) -> String {
    let bytes =
        fs::read(data.join("modn/buckets/main/bucket.json")).expect("installed bucket metadata");
    let metadata: Value = serde_json::from_slice(&bytes).expect("installed bucket JSON");
    metadata["title"]
        .as_str()
        .expect("installed bucket title")
        .to_owned()
}

fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[test]
fn newest_supported_release_is_installed_and_a_newer_release_is_reported() {
    let data = tempfile::tempdir().expect("data directory");
    let fixtures = tempfile::tempdir().expect("release fixtures");
    let supported = fixture_tree("Supported");
    let future = fixture_tree("Future");
    let newer = release(
        fixtures.path(),
        "v2026.12.01",
        future.path(),
        Some(r#"{"schema_version":2,"min_modn_version":"99.0.0"}"#),
    );
    let older = release(
        fixtures.path(),
        "v2026.11.24",
        supported.path(),
        Some(r#"{"schema_version":2,"min_modn_version":"0.0.1"}"#),
    );
    let list = release_list(fixtures.path(), &[newer, older]);

    let text = success(run(data.path(), &list, &["bucket", "update"]));
    let stderr = String::from_utf8_lossy(&text.stderr);
    assert!(
        stderr.contains(
            "main release v2026.12.01 needs Modn 99.0.0; keeping v2026.11.24. Update Modn to get it."
        ),
        "{stderr}"
    );
    assert_eq!(installed_title(data.path()), "Supported");

    let json = success(run(data.path(), &list, &["--json", "bucket", "update"]));
    let report: Value = serde_json::from_slice(&json.stdout).expect("bucket update JSON");
    assert_eq!(report["release"], "v2026.11.24", "{report}");
    assert_eq!(report["newer_release_needs_modn"], "99.0.0", "{report}");
    assert_eq!(report["changed"], false, "{report}");
}

#[test]
fn a_release_without_the_requires_asset_is_readable() {
    let data = tempfile::tempdir().expect("data directory");
    let fixtures = tempfile::tempdir().expect("release fixtures");
    let legacy = fixture_tree("Legacy");
    let only = release(fixtures.path(), "v2026.11.24", legacy.path(), None);
    let list = release_list(fixtures.path(), &[only]);

    let text = success(run(data.path(), &list, &["bucket", "update"]));
    let stderr = String::from_utf8_lossy(&text.stderr);
    assert!(!stderr.contains("needs Modn"), "{stderr}");
    assert_eq!(installed_title(data.path()), "Legacy");

    let json = success(run(data.path(), &list, &["--json", "bucket", "update"]));
    let report: Value = serde_json::from_slice(&json.stdout).expect("bucket update JSON");
    assert_eq!(report["release"], "v2026.11.24", "{report}");
    assert!(report["newer_release_needs_modn"].is_null(), "{report}");
}

#[test]
fn a_malformed_requires_file_skips_that_release() {
    let data = tempfile::tempdir().expect("data directory");
    let fixtures = tempfile::tempdir().expect("release fixtures");
    let broken_json = fixture_tree("Broken JSON");
    let broken_version = fixture_tree("Broken version");
    let legacy = fixture_tree("Legacy");
    let newest = release(
        fixtures.path(),
        "v2026.12.01",
        broken_json.path(),
        Some("{not json"),
    );
    let second = release(
        fixtures.path(),
        "v2026.12.02",
        broken_version.path(),
        Some(r#"{"schema_version":2,"min_modn_version":"latest"}"#),
    );
    let older = release(fixtures.path(), "v2026.11.24", legacy.path(), None);
    let list = release_list(fixtures.path(), &[newest, second, older]);

    let output = success(run(data.path(), &list, &["bucket", "update"]));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Skipping main release v2026.12.01: invalid main-bucket.requires.json"),
        "{stderr}"
    );
    assert!(
        stderr.contains(r#"Skipping main release v2026.12.02: invalid min_modn_version "latest""#),
        "{stderr}"
    );
    assert!(!stderr.contains("needs Modn"), "{stderr}");
    assert_eq!(installed_title(data.path()), "Legacy");
}

#[test]
fn a_requires_asset_read_failure_fails_instead_of_selecting_an_older_release() {
    let data = tempfile::tempdir().expect("data directory");
    let fixtures = tempfile::tempdir().expect("release fixtures");
    let future = fixture_tree("Future");
    let legacy = fixture_tree("Legacy");
    let newer = release(
        fixtures.path(),
        "v2026.12.01",
        future.path(),
        Some(r#"{"schema_version":2,"min_modn_version":"99.0.0"}"#),
    );
    fs::remove_file(
        fixtures
            .path()
            .join("v2026.12.01/main-bucket.requires.json"),
    )
    .expect("remove requires asset");
    let older = release(fixtures.path(), "v2026.11.24", legacy.path(), None);
    let list = release_list(fixtures.path(), &[newer, older]);

    let output = run(data.path(), &list, &["bucket", "update"]);
    assert!(!output.status.success(), "the command must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot read main-bucket.requires.json"),
        "{stderr}"
    );
    assert!(!data.path().join("modn/buckets/main").exists());
}

#[test]
fn an_unsupported_bucket_schema_is_skipped_and_reported() {
    let data = tempfile::tempdir().expect("data directory");
    let fixtures = tempfile::tempdir().expect("release fixtures");
    let future = fixture_tree("Future schema");
    let supported = fixture_tree("Supported");
    let newer = release(
        fixtures.path(),
        "v2026.12.01",
        future.path(),
        Some(r#"{"schema_version":3,"min_modn_version":"0.0.1"}"#),
    );
    let older = release(
        fixtures.path(),
        "v2026.11.24",
        supported.path(),
        Some(r#"{"schema_version":2,"min_modn_version":"0.0.1"}"#),
    );
    let list = release_list(fixtures.path(), &[newer, older]);

    let text = success(run(data.path(), &list, &["bucket", "update"]));
    let stderr = String::from_utf8_lossy(&text.stderr);
    assert!(
        stderr.contains(
            "main release v2026.12.01 needs Modn 0.0.1 and bucket contract v3; keeping v2026.11.24. Update Modn to get it."
        ),
        "{stderr}"
    );
    assert_eq!(installed_title(data.path()), "Supported");

    let json = success(run(data.path(), &list, &["--json", "bucket", "update"]));
    let report: Value = serde_json::from_slice(&json.stdout).expect("bucket update JSON");
    assert_eq!(report["release"], "v2026.11.24", "{report}");
    assert_eq!(report["newer_release_needs_modn"], "0.0.1", "{report}");
}

#[test]
fn drafts_and_prereleases_are_not_selected_or_reported_as_newer() {
    let data = tempfile::tempdir().expect("data directory");
    let fixtures = tempfile::tempdir().expect("release fixtures");
    let future = fixture_tree("Future");
    let stable = fixture_tree("Stable");
    let mut draft = release(
        fixtures.path(),
        "v2026.12.02",
        future.path(),
        Some(r#"{"schema_version":2,"min_modn_version":"99.0.0"}"#),
    );
    draft["draft"] = serde_json::json!(true);
    let mut prerelease = release(
        fixtures.path(),
        "v2026.12.01",
        future.path(),
        Some(r#"{"schema_version":2,"min_modn_version":"99.0.0"}"#),
    );
    prerelease["prerelease"] = serde_json::json!(true);
    let older = release(fixtures.path(), "v2026.11.24", stable.path(), None);
    let list = release_list(fixtures.path(), &[draft, prerelease, older]);

    let text = success(run(data.path(), &list, &["bucket", "update"]));
    let stderr = String::from_utf8_lossy(&text.stderr);
    assert!(!stderr.contains("needs Modn"), "{stderr}");
    assert_eq!(installed_title(data.path()), "Stable");

    let json = success(run(data.path(), &list, &["--json", "bucket", "update"]));
    let report: Value = serde_json::from_slice(&json.stdout).expect("bucket update JSON");
    assert_eq!(report["release"], "v2026.11.24", "{report}");
    assert!(report["newer_release_needs_modn"].is_null(), "{report}");
}

#[test]
fn no_supported_release_fails_and_keeps_the_installed_bucket() {
    let data = tempfile::tempdir().expect("data directory");
    let fixtures = tempfile::tempdir().expect("release fixtures");
    let legacy = fixture_tree("Legacy");
    let supported = release(fixtures.path(), "v2026.11.24", legacy.path(), None);
    let installed = release_list(fixtures.path(), &[supported]);
    success(run(data.path(), &installed, &["bucket", "update"]));
    let main = data.path().join("modn/buckets/main");
    let before = modn_core::bucket_digest::directory_digest(&main).expect("installed digest");

    let future = fixture_tree("Future");
    let newer = release(
        fixtures.path(),
        "v2026.12.01",
        future.path(),
        Some(r#"{"schema_version":2,"min_modn_version":"99.0.0"}"#),
    );
    let unsupported = release_list(fixtures.path(), &[newer]);
    let output = run(data.path(), &unsupported, &["bucket", "update"]);
    assert!(!output.status.success(), "the command must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(&format!(
            "no main bucket release is supported by Modn {}",
            version()
        )),
        "{stderr}"
    );
    assert!(stderr.contains("needs Modn 99.0.0"), "{stderr}");
    assert_eq!(
        modn_core::bucket_digest::directory_digest(&main).expect("installed digest"),
        before
    );
}
