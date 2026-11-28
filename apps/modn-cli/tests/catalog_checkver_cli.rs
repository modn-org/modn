#[path = "support/checkver_fixture.rs"]
mod checkver_fixture;
#[path = "support/git_fixture.rs"]
mod git_fixture;
#[path = "support/main_bucket.rs"]
mod main_bucket;

use checkver_fixture::{
    archive_manifest, git_manifest, github_route, github_source, json_output, snapshot, stderr,
    stdout, Fixture, MockServer,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;

#[test]
fn checkver_github_release_reports_update_and_up_to_date_with_exact_json() {
    let fixture = Fixture::new();
    let mut routes = BTreeMap::new();
    let (path, route) = github_route(
        "owner/alpha",
        json!([
            {"tag_name": "v1.2.0", "assets": [{"name": "Alpha-1.2.0.zip"}]},
            {"tag_name": "v1.0.0", "assets": [{"name": "Alpha-1.0.0.zip"}]}
        ]),
    );
    routes.insert(path, route);
    let (path, route) = github_route(
        "owner/beta",
        json!([{"tag_name": "v1.5.0", "assets": [{"name": "Beta-1.5.0.zip"}]}]),
    );
    routes.insert(path, route);
    let server = MockServer::new(routes);
    fixture.write(
        "test-game",
        "alpha",
        &archive_manifest(
            "test-game",
            "alpha",
            "1.0.0",
            github_source("owner/alpha", "v1.0.0", "Alpha-1.0.0.zip"),
            Some(json!({
                "kind": "github-release",
                "repository": "owner/alpha",
                "asset_pattern": "Alpha-*.zip"
            })),
        ),
    );
    fixture.write(
        "test-game",
        "beta",
        &archive_manifest(
            "test-game",
            "beta",
            "2.0.0",
            github_source("owner/beta", "v2.0.0", "Beta-2.0.0.zip"),
            Some(json!({
                "kind": "github-release",
                "repository": "owner/beta",
                "asset_pattern": "Beta-*.zip"
            })),
        ),
    );
    let before = snapshot(&fixture.catalog);
    let env = [
        ("MODN_TEST_CHECKVER_GITHUB_API", server.address.as_str()),
        ("GITHUB_TOKEN", "fixture-token"),
    ];

    let human = fixture.run(&env, &["catalog", "checkver"]);
    assert!(
        human.status.success(),
        "{}\n{}",
        stderr(&human),
        stdout(&human)
    );
    let text = stdout(&human);
    assert!(
        text.contains("test-game/alpha: update_available 1.0.0 -> 1.2.0"),
        "{text}"
    );
    assert!(
        text.contains("test-game/beta: up_to_date 2.0.0 -> 1.5.0"),
        "{text}"
    );
    assert!(
        text.contains("Summary: 1 up_to_date, 1 update_available, 0 skipped, 0 error"),
        "{text}"
    );

    let json_run = fixture.run(&env, &["--json", "catalog", "checkver"]);
    assert!(
        json_run.status.success(),
        "{}\n{}",
        stderr(&json_run),
        stdout(&json_run)
    );
    let value = json_output(&json_run);
    assert_eq!(
        value["packages"],
        json!([
            {"game": "test-game", "id": "alpha", "status": "update_available",
             "kind": "github-release", "current_version": "1.0.0",
             "latest_version": "1.2.0", "error": null},
            {"game": "test-game", "id": "beta", "status": "up_to_date",
             "kind": "github-release", "current_version": "2.0.0",
             "latest_version": "1.5.0", "error": null}
        ])
    );
    assert_eq!(
        value["summary"],
        json!({"up_to_date": 1, "update_available": 1, "skipped": 0, "error": 0})
    );
    assert_eq!(stdout(&json_run).lines().count(), 1);
    assert_eq!(before, snapshot(&fixture.catalog), "checkver wrote a file");
    assert!(
        !fixture.temp.path().join("state").exists(),
        "checkver created manager state"
    );
    assert!(
        !stderr(&human).contains("fixture-token"),
        "the token leaked"
    );
    assert_eq!(
        server.header("/repos/owner/alpha/", "authorization"),
        None,
        "GITHUB_TOKEN went to a non-GitHub host"
    );
}

#[test]
fn checkver_prerelease_flag_and_git_branch_are_honoured() {
    let fixture = Fixture::new();
    let repository = git_fixture::repository(fixture.temp.path());
    let mut routes = BTreeMap::new();
    let (path, route) = github_route(
        "owner/preview",
        json!([
            {"tag_name": "v3.0.0-beta.1", "prerelease": true,
             "assets": [{"name": "Preview-3.0.0.zip"}]},
            {"tag_name": "v2.0.0", "assets": [{"name": "Preview-2.0.0.zip"}]}
        ]),
    );
    routes.insert(path, route);
    let server = MockServer::new(routes);
    fixture.write(
        "test-game",
        "preview",
        &archive_manifest(
            "test-game",
            "preview",
            "2.5.0",
            github_source("owner/preview", "v2.5.0", "Preview-2.5.0.zip"),
            Some(json!({
                "kind": "github-release",
                "repository": "owner/preview",
                "asset_pattern": "Preview-*.zip",
                "prerelease": true
            })),
        ),
    );
    fixture.write(
        "test-game",
        "stable",
        &archive_manifest(
            "test-game",
            "stable",
            "2.5.0",
            github_source("owner/preview", "v2.5.0", "Preview-2.5.0.zip"),
            Some(json!({
                "kind": "github-release",
                "repository": "owner/preview",
                "asset_pattern": "Preview-*.zip"
            })),
        ),
    );
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
    fixture.write(
        "test-game",
        "git-current",
        &git_manifest(
            "test-game",
            "git-current",
            "1.0",
            &repository.later,
            json!({"kind": "git", "repository": "fixture/project", "branch": "main"}),
        ),
    );
    let host = git_fixture::file_url(&repository.base);
    let env = [
        ("MODN_TEST_CHECKVER_GITHUB_API", server.address.as_str()),
        ("MODN_TEST_CHECKVER_GITHUB_HOST", host.as_str()),
    ];
    let output = fixture.run(&env, &["--json", "catalog", "checkver"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let value = json_output(&output);
    let packages = value["packages"].as_array().expect("packages");
    assert_eq!(packages[0]["status"], "up_to_date", "{value}");
    assert_eq!(packages[1]["status"], "update_available", "{value}");
    assert_eq!(
        packages[1]["latest_version"],
        format!("git.{}", repository.later)
    );
    assert_eq!(
        packages[2]["status"], "update_available",
        "prerelease flag was ignored: {value}"
    );
    assert_eq!(packages[2]["latest_version"], "3.0.0-beta.1");
    assert_eq!(
        packages[3]["status"], "up_to_date",
        "a prerelease satisfied a stable check: {value}"
    );
    assert_eq!(packages[3]["latest_version"], "2.0.0");
}

#[test]
fn checkver_skips_plain_packages_reports_partial_failure_and_targets_one() {
    let fixture = Fixture::new();
    let mut routes = BTreeMap::new();
    let (path, route) = github_route(
        "owner/alpha",
        json!([{"tag_name": "v1.1.0", "assets": [{"name": "Alpha.zip"}]}]),
    );
    routes.insert(path, route);
    let server = MockServer::new(routes);
    fixture.write(
        "test-game",
        "alpha",
        &archive_manifest(
            "test-game",
            "alpha",
            "1.0.0",
            github_source("owner/alpha", "v1.0.0", "Alpha.zip"),
            Some(json!({
                "kind": "github-release",
                "repository": "owner/alpha",
                "asset_pattern": "Alpha.zip"
            })),
        ),
    );
    fixture.write(
        "test-game",
        "broken",
        &archive_manifest(
            "test-game",
            "broken",
            "1.0.0",
            github_source("owner/missing", "v1.0.0", "Broken.zip"),
            Some(json!({
                "kind": "github-release",
                "repository": "owner/missing",
                "asset_pattern": "Broken.zip"
            })),
        ),
    );
    fixture.write(
        "test-game",
        "plain",
        &archive_manifest(
            "test-game",
            "plain",
            "1.0.0",
            json!({"kind": "manual", "filename": "plain.zip"}),
            None,
        ),
    );
    fixture.write("test-game", "malformed", &Value::Null);
    fs::write(
        fixture.catalog.join("mods/test-game/alpha@0.9.json"),
        b"{not JSON",
    )
    .expect("old release");
    let env = [("MODN_TEST_CHECKVER_GITHUB_API", server.address.as_str())];

    let all = fixture.run(&env, &["catalog", "checkver"]);
    assert!(
        !all.status.success(),
        "a package error did not fail the run: {}",
        stdout(&all)
    );
    let text = stdout(&all);
    assert!(
        text.contains("test-game/alpha: update_available 1.0.0 -> 1.1.0"),
        "{text}"
    );
    assert!(text.contains("test-game/broken: error:"), "{text}");
    assert!(text.contains("HTTP status 404"), "{text}");
    assert!(text.contains("test-game/plain: skipped 1.0.0"), "{text}");
    assert!(
        text.contains("0 up_to_date, 1 update_available, 1 skipped, 2 error"),
        "{text}"
    );

    let plain = fixture.run_target(&env, &["catalog", "checkver"], "plain");
    assert!(plain.status.success(), "{}", stderr(&plain));
    assert_eq!(
        stdout(&plain),
        "test-game/plain: skipped 1.0.0\nSummary: 0 up_to_date, 0 update_available, 1 skipped, 0 error\n"
    );

    let one = fixture.run_target(&env, &["catalog", "checkver"], "test-game/alpha");
    assert!(one.status.success(), "{}", stderr(&one));
    assert!(stdout(&one).starts_with("test-game/alpha: update_available"));

    let unknown = fixture.run_target(&env, &["catalog", "checkver"], "nope");
    assert!(!unknown.status.success());
    assert!(stderr(&unknown).contains("catalog package not found: nope"));

    fixture.write(
        "other-game",
        "plain",
        &archive_manifest(
            "other-game",
            "plain",
            "1.0.0",
            json!({"kind": "manual", "filename": "plain.zip"}),
            None,
        ),
    );
    let ambiguous = fixture.run_target(&env, &["catalog", "checkver"], "plain");
    assert!(!ambiguous.status.success());
    let message = stderr(&ambiguous);
    assert!(message.contains("ambiguous"), "{message}");
    assert!(message.contains("other-game"), "{message}");
}

#[test]
fn checkver_nexus_missing_key_and_update_available_through_the_test_seam() {
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
                {"file_id": 200, "file_name": "Mod-2.0.zip", "uploaded_timestamp": 200},
                {"file_id": 300, "file_name": "Other.zip", "uploaded_timestamp": 300}
            ]))
            .expect("files JSON"),
        ),
    );
    let server = MockServer::new(routes);
    let missing = fixture.run(
        &[
            ("MODN_TEST_CHECKVER_NEXUS_API", server.address.as_str()),
            ("MODN_TEST_CHECKVER_NEXUS_KEY", "missing"),
        ],
        &["--json", "catalog", "checkver"],
    );
    assert!(!missing.status.success());
    let value = json_output(&missing);
    assert_eq!(value["packages"][0]["status"], "error");
    assert_eq!(value["packages"][0]["kind"], "nexus");
    assert!(
        value["packages"][0]["error"]
            .as_str()
            .expect("error text")
            .contains("Nexus Mods needs an API key"),
        "{value}"
    );
    assert_eq!(server.requests().len(), 0, "missing key made a request");

    let updated = fixture.run(
        &[
            ("MODN_TEST_CHECKVER_NEXUS_API", server.address.as_str()),
            ("MODN_TEST_CHECKVER_NEXUS_KEY", "fixture-nexus-key"),
        ],
        &["--json", "catalog", "checkver"],
    );
    assert!(updated.status.success(), "{}", stderr(&updated));
    let value = json_output(&updated);
    assert_eq!(
        value["packages"][0]["status"], "update_available",
        "{value}"
    );
    assert_eq!(value["packages"][0]["latest_version"], Value::Null);
    assert_eq!(
        server.header("/v1/games/cyberpunk2077/", "apikey"),
        Some("fixture-nexus-key".to_owned())
    );
    assert!(!stdout(&updated).contains("fixture-nexus-key"));
    assert!(!stderr(&updated).contains("fixture-nexus-key"));
}
