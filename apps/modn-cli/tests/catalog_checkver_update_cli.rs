#[path = "support/checkver_fixture.rs"]
mod checkver_fixture;
#[path = "support/checkver_update_fixture.rs"]
mod checkver_update_fixture;
#[path = "support/git_fixture.rs"]
mod git_fixture;
#[path = "support/main_bucket.rs"]
mod main_bucket;

use checkver_fixture::{
    archive_manifest, git_manifest, github_route, json_output, snapshot, stderr, stdout, Fixture,
    MockServer,
};
use checkver_update_fixture::{
    add_release, directory_entries, manifest_text, server_env, sign_and_verify, zeros, REMINDER,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;

#[test]
fn checkver_update_rewrites_a_github_release_and_keeps_unrelated_bytes() {
    let fixture = Fixture::new();
    let bytes = b"new upstream archive bytes";
    let mut routes = BTreeMap::new();
    add_release(
        &mut routes,
        "alpha",
        "v1.2.0",
        "Alpha-1.2.0.zip",
        Some(bytes),
    );
    let server = MockServer::new(routes);
    let old = manifest_text(
        "alpha",
        "1.0.0",
        "v1.0.0",
        "Alpha-1.0.0.zip",
        &zeros(),
        Some(&"b".repeat(64)),
    );
    fixture.write_text("test-game", "alpha", &old);
    fs::write(fixture.catalog.join("catalog.pub"), b"untouched public key")
        .expect("public key fixture");
    let signature = fs::read(fixture.catalog.join("catalog.sig")).expect("signature");
    let public_key = fs::read(fixture.catalog.join("catalog.pub")).expect("public key");
    let env = server_env(&server);
    let before = snapshot(&fixture.catalog);
    let check_only = fixture.run(&env, &["catalog", "checkver"]);
    assert!(check_only.status.success(), "{}", stderr(&check_only));
    assert!(
        stdout(&check_only).contains("test-game/alpha: update_available 1.0.0 -> 1.2.0"),
        "{}",
        stdout(&check_only)
    );
    assert_eq!(
        before,
        snapshot(&fixture.catalog),
        "check-only wrote a catalog file"
    );

    let output = fixture.run(&env, &["catalog", "checkver", "--update"]);
    assert!(
        output.status.success(),
        "{}\n{}",
        stderr(&output),
        stdout(&output)
    );
    let hash = blake3::hash(bytes).to_hex().to_string();
    let downloaded = fixture.temp.path().join("downloaded.zip");
    fs::write(&downloaded, bytes).expect("downloaded bytes");
    let hashed = fixture.run_plain(
        &[],
        &[
            "catalog",
            "hash",
            downloaded.to_str().expect("download path"),
        ],
    );
    assert!(hashed.status.success(), "{}", stderr(&hashed));
    assert_eq!(stdout(&hashed).trim(), hash);
    let expected = manifest_text("alpha", "1.2.0", "v1.2.0", "Alpha-1.2.0.zip", &hash, None);
    assert_eq!(
        fs::read_to_string(fixture.manifest_path("test-game", "alpha")).expect("manifest"),
        expected,
        "the rewrite changed more than the values checkver updates"
    );
    let text = stdout(&output);
    assert!(
        text.contains("test-game/alpha: update_available 1.0.0 -> 1.2.0"),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "updated test-game/alpha: 1.0.0 -> 1.2.0 (blake3 {}...)",
            &hash[..12]
        )),
        "{text}"
    );
    assert!(
        text.contains("warning: test-game/alpha: removed upstream_sha256"),
        "{text}"
    );
    assert!(text.contains(REMINDER), "{text}");
    assert_eq!(
        fs::read(fixture.catalog.join("catalog.sig")).expect("signature"),
        signature,
        "checkver changed catalog.sig"
    );
    assert_eq!(
        fs::read(fixture.catalog.join("catalog.pub")).expect("public key"),
        public_key,
        "checkver changed catalog.pub"
    );
    assert_eq!(
        directory_entries(&fixture.catalog.join("mods/test-game")),
        ["alpha.json"],
        "checkver left a temporary or release file"
    );

    let check = fixture.run(&env, &["catalog", "checkver"]);
    assert!(check.status.success(), "{}", stderr(&check));
    assert!(
        stdout(&check).contains("test-game/alpha: up_to_date 1.2.0 -> 1.2.0"),
        "{}",
        stdout(&check)
    );

    sign_and_verify(&fixture);
}

#[test]
fn checkver_update_keep_previous_copies_the_old_manifest_and_refuses_a_conflict() {
    let fixture = Fixture::new();
    let bytes = b"newer bytes";
    let mut routes = BTreeMap::new();
    add_release(
        &mut routes,
        "alpha",
        "v1.2.0",
        "Alpha-1.2.0.zip",
        Some(bytes),
    );
    let server = MockServer::new(routes);
    let old = manifest_text(
        "alpha",
        "1.0.0",
        "v1.0.0",
        "Alpha-1.0.0.zip",
        &zeros(),
        None,
    );
    fixture.write_text("test-game", "alpha", &old);
    let env = server_env(&server);

    let output = fixture.run(
        &env,
        &["catalog", "checkver", "--update", "--keep-previous"],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("kept previous manifest: mods/test-game/alpha@1.0.0.json"),
        "{}",
        stdout(&output)
    );
    assert_eq!(
        fs::read(fixture.catalog.join("mods/test-game/alpha@1.0.0.json")).expect("old manifest"),
        old.as_bytes()
    );

    let conflict = Fixture::new();
    let mut routes = BTreeMap::new();
    add_release(
        &mut routes,
        "alpha",
        "v1.2.0",
        "Alpha-1.2.0.zip",
        Some(bytes),
    );
    let server = MockServer::new(routes);
    conflict.write_text("test-game", "alpha", &old);
    fs::write(
        conflict.catalog.join("mods/test-game/alpha@1.0.0.json"),
        b"different bytes",
    )
    .expect("conflicting release");
    let env = server_env(&server);
    let before = snapshot(&conflict.catalog);
    let output = conflict.run(
        &env,
        &["catalog", "checkver", "--update", "--keep-previous"],
    );
    assert!(!output.status.success(), "{}", stdout(&output));
    assert!(
        stdout(&output).contains("previous release manifest already exists with different bytes"),
        "{}",
        stdout(&output)
    );
    assert!(!stdout(&output).contains("updated test-game/alpha:"));
    assert_eq!(
        before,
        snapshot(&conflict.catalog),
        "a conflict wrote a file"
    );

    let usage = fixture.run(&env, &["catalog", "checkver", "--keep-previous"]);
    assert!(!usage.status.success());
    assert!(
        stderr(&usage).contains("--keep-previous") && stderr(&usage).contains("--update"),
        "{}",
        stderr(&usage)
    );
}

#[test]
fn checkver_update_isolates_a_failed_download_and_updates_the_good_package() {
    let fixture = Fixture::new();
    let bytes = b"good bytes";
    let mut routes = BTreeMap::new();
    add_release(
        &mut routes,
        "alpha",
        "v1.2.0",
        "Alpha-1.2.0.zip",
        Some(bytes),
    );
    add_release(&mut routes, "broken", "v1.2.0", "Broken-1.2.0.zip", None);
    let server = MockServer::new(routes);
    fixture.write_text(
        "test-game",
        "alpha",
        &manifest_text(
            "alpha",
            "1.0.0",
            "v1.0.0",
            "Alpha-1.0.0.zip",
            &zeros(),
            None,
        ),
    );
    let broken = manifest_text(
        "broken",
        "1.0.0",
        "v1.0.0",
        "Broken-1.0.0.zip",
        &zeros(),
        None,
    );
    fixture.write_text("test-game", "broken", &broken);
    let env = server_env(&server);

    let output = fixture.run(&env, &["catalog", "checkver", "--update"]);
    assert!(!output.status.success(), "{}", stdout(&output));
    let text = stdout(&output);
    assert!(
        text.contains("updated test-game/alpha: 1.0.0 -> 1.2.0"),
        "{text}"
    );
    assert!(text.contains("test-game/broken: error:"), "{text}");
    assert!(text.contains("HTTP status 404"), "{text}");
    assert!(
        text.contains("Summary: 0 up_to_date, 1 update_available, 0 skipped, 1 error"),
        "{text}"
    );
    assert_eq!(
        fs::read_to_string(fixture.manifest_path("test-game", "broken")).expect("broken manifest"),
        broken,
        "a failed package changed its manifest"
    );
    assert_eq!(
        directory_entries(&fixture.catalog.join("mods/test-game")),
        ["alpha.json", "broken.json"],
        "checkver left a temporary file"
    );
}

#[test]
fn checkver_update_json_adds_only_the_documented_fields() {
    let fixture = Fixture::new();
    let bytes = b"json bytes";
    let mut routes = BTreeMap::new();
    add_release(
        &mut routes,
        "alpha",
        "v1.2.0",
        "Alpha-1.2.0.zip",
        Some(bytes),
    );
    let (path, route) = github_route(
        "owner/beta",
        json!([{"tag_name": "v2.0.0", "assets": [{"name": "Beta-2.0.0.zip"}]}]),
    );
    routes.insert(path, route);
    let server = MockServer::new(routes);
    fixture.write_text(
        "test-game",
        "alpha",
        &manifest_text(
            "alpha",
            "1.0.0",
            "v1.0.0",
            "Alpha-1.0.0.zip",
            &zeros(),
            None,
        ),
    );
    fixture.write_text(
        "test-game",
        "beta",
        &manifest_text("beta", "2.0.0", "v2.0.0", "Beta-2.0.0.zip", &zeros(), None),
    );
    let env = server_env(&server);

    let output = fixture.run(
        &env,
        &[
            "--json",
            "catalog",
            "checkver",
            "--update",
            "--keep-previous",
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output).lines().count(), 1);
    let value = json_output(&output);
    assert_eq!(value["packages"][0]["status"], "update_available");
    assert_eq!(
        value["packages"][0]["kept_previous"],
        "mods/test-game/alpha@1.0.0.json"
    );
    assert_eq!(value["packages"][0]["updated"], true);
    assert_eq!(value["packages"][1]["updated"], false);
    assert_eq!(value["packages"][1]["kept_previous"], Value::Null);
    assert_eq!(value["summary"]["updated"], 1);
    assert_eq!(value["reminder"], REMINDER);

    let check = fixture.run(&env, &["--json", "catalog", "checkver"]);
    assert!(check.status.success(), "{}", stderr(&check));
    let value = json_output(&check);
    let package = value["packages"][0].as_object().expect("package object");
    let mut keys: Vec<&str> = package.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "current_version",
            "error",
            "game",
            "id",
            "kind",
            "latest_version",
            "status"
        ]
    );
    assert!(
        value["summary"].get("updated").is_none(),
        "check-only JSON gained a summary field: {value}"
    );
    assert!(
        value.get("reminder").is_none(),
        "check-only JSON gained a reminder"
    );
}

#[test]
fn checkver_update_rewrites_a_git_commit_and_version() {
    let fixture = Fixture::new();
    let repository = git_fixture::repository(fixture.temp.path());
    fixture.write(
        "test-game",
        "git-newer",
        &git_manifest(
            "test-game",
            "git-newer",
            "1.0",
            &repository.commit,
            json!({"kind": "git", "repository": "fixture/project", "branch": "main"}),
        ),
    );
    let host = git_fixture::file_url(&repository.base);
    let env = [("MODN_TEST_CHECKVER_GITHUB_HOST", host.as_str())];

    let output = fixture.run(&env, &["catalog", "checkver", "--update"]);
    assert!(
        output.status.success(),
        "{}\n{}",
        stderr(&output),
        stdout(&output)
    );
    let manifest: Value = serde_json::from_slice(
        &fs::read(fixture.manifest_path("test-game", "git-newer")).expect("manifest"),
    )
    .expect("manifest JSON");
    assert_eq!(
        manifest["version"],
        format!("git.{}", repository.later),
        "{manifest}"
    );
    assert_eq!(manifest["sources"][0]["commit"], repository.later);
    assert!(
        manifest.get("blake3").is_none(),
        "a git update wrote blake3: {manifest}"
    );
    let text = stdout(&output);
    assert!(
        text.contains(&format!(
            "updated test-game/git-newer: 1.0 -> git.{} (commit {}...)",
            repository.later,
            &repository.later[..12]
        )),
        "{text}"
    );
    assert!(text.contains(REMINDER), "{text}");

    let check = fixture.run(&env, &["catalog", "checkver"]);
    assert!(check.status.success(), "{}", stderr(&check));
    assert!(stdout(&check).contains("up_to_date"), "{}", stdout(&check));
}

#[test]
fn checkver_update_refuses_nexus_without_writing() {
    let fixture = Fixture::new();
    fixture.write(
        "test-game",
        "nexus-mod",
        &archive_manifest(
            "test-game",
            "nexus-mod",
            "1.0",
            json!({"kind": "nexus", "game_domain": "cyberpunk2077", "mod_id": 6945,
                   "file_id": 100, "file_name": "Mod-1.0.zip"}),
            Some(json!({"kind": "nexus", "game_domain": "cyberpunk2077",
                        "mod_id": 6945, "file_pattern": "Mod-*.zip"})),
        ),
    );
    let mut routes = BTreeMap::new();
    routes.insert(
        "/v1/games/cyberpunk2077/mods/6945/files.json".to_owned(),
        (
            200,
            serde_json::to_vec(&json!([
                {"file_id": 100, "file_name": "Mod-1.0.zip", "uploaded_timestamp": 100},
                {"file_id": 200, "file_name": "Mod-2.0.zip", "uploaded_timestamp": 200}
            ]))
            .expect("files JSON"),
        ),
    );
    let server = MockServer::new(routes);
    let before = snapshot(&fixture.catalog);
    let env = [
        ("MODN_TEST_CHECKVER_NEXUS_API", server.address.as_str()),
        ("MODN_TEST_CHECKVER_NEXUS_KEY", "fixture-nexus-key"),
    ];

    let output = fixture.run(&env, &["catalog", "checkver", "--update"]);
    assert!(!output.status.success(), "{}", stdout(&output));
    assert!(
        stdout(&output)
            .contains("test-game/nexus-mod: error: nexus checkver update is not supported: update the manifest by hand"),
        "{}",
        stdout(&output)
    );
    assert_eq!(
        before,
        snapshot(&fixture.catalog),
        "nexus update wrote a file"
    );
    assert_eq!(
        server.requests(),
        ["/v1/games/cyberpunk2077/mods/6945/files.json"],
        "nexus update downloaded a file"
    );
    assert!(!stdout(&output).contains("fixture-nexus-key"));
}
