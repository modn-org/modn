use super::Checkver;
use crate::buckets::PackageManifest;
use anyhow::{Context, Result};
use serde_json::{json, Value};

fn base_manifest() -> Value {
    json!({
        "schema_version": 2,
        "id": "top",
        "game": "test-game",
        "name": "Top",
        "version": "1.0",
        "archive_format": "zip",
        "blake3": "0".repeat(64),
        "sources": [{"kind": "manual", "filename": "top.zip"}]
    })
}

fn parse(checkver: Value) -> Result<PackageManifest> {
    let mut manifest = base_manifest();
    manifest["checkver"] = checkver;
    serde_json::from_value(manifest).with_context(|| "package test-game/top")
}

fn validate(checkver: Value) -> Result<()> {
    parse(checkver)?.validate("test-game", "top")
}

#[test]
fn checkver_parses_and_validates_all_three_kinds() {
    let github = parse(json!({
        "kind": "github-release",
        "repository": "owner/repo",
        "asset_pattern": "Mod-*.zip",
        "prerelease": true
    }))
    .expect("valid github-release checkver");
    assert!(matches!(
        github.checkver,
        Some(Checkver::GithubRelease {
            ref repository,
            ref asset_pattern,
            prerelease: true,
        }) if repository == "owner/repo" && asset_pattern == "Mod-*.zip"
    ));
    github
        .validate("test-game", "top")
        .expect("valid github-release manifest");

    let git = parse(json!({
        "kind": "git",
        "repository": "owner/repo",
        "branch": "release/1.0"
    }))
    .expect("valid git checkver");
    assert!(matches!(
        git.checkver,
        Some(Checkver::Git {
            ref repository,
            ref branch,
        }) if repository == "owner/repo" && branch == "release/1.0"
    ));
    git.validate("test-game", "top")
        .expect("valid git manifest");

    let nexus = parse(json!({
        "kind": "nexus",
        "game_domain": "cyberpunk2077",
        "mod_id": 6945,
        "file_pattern": "Mod-?.7z"
    }))
    .expect("valid nexus checkver");
    assert!(matches!(
        nexus.checkver,
        Some(Checkver::Nexus {
            ref game_domain,
            mod_id: 6945,
            ref file_pattern,
        }) if game_domain == "cyberpunk2077" && file_pattern == "Mod-?.7z"
    ));
    nexus
        .validate("test-game", "top")
        .expect("valid nexus manifest");
}

#[test]
fn checkver_prerelease_defaults_to_false_and_is_omitted_when_false() {
    let manifest = parse(json!({
        "kind": "github-release",
        "repository": "owner/repo",
        "asset_pattern": "Mod-*.zip"
    }))
    .expect("valid default");
    assert!(matches!(
        manifest.checkver,
        Some(Checkver::GithubRelease {
            prerelease: false,
            ..
        })
    ));
    let serialized = serde_json::to_value(&manifest).expect("serialize manifest");
    assert!(serialized["checkver"].get("prerelease").is_none());
}

#[test]
fn manifests_without_checkver_parse_and_validate_unchanged() {
    let manifest: PackageManifest =
        serde_json::from_value(base_manifest()).expect("legacy contract v2 manifest");
    assert!(manifest.checkver.is_none());
    manifest
        .validate("test-game", "top")
        .expect("unchanged validation");
    let serialized = serde_json::to_value(&manifest).expect("serialize manifest");
    assert!(serialized.get("checkver").is_none());
}

#[test]
fn checkver_rejects_unknown_kinds_fields_and_missing_fields() {
    for checkver in [
        json!({"kind": "svn", "repository": "owner/repo", "asset_pattern": "Mod-*.zip"}),
        json!({"kind": "github-release", "repository": "owner/repo"}),
        json!({"kind": "github-release", "asset_pattern": "Mod-*.zip"}),
        json!({"kind": "git", "repository": "owner/repo"}),
        json!({"kind": "git", "branch": "main"}),
        json!({"kind": "nexus", "mod_id": 6945, "file_pattern": "Mod.zip"}),
        json!({"kind": "nexus", "game_domain": "cyberpunk2077", "file_pattern": "Mod.zip"}),
        json!({
            "kind": "nexus", "game_domain": "cyberpunk2077", "mod_id": 6945
        }),
        json!({
            "kind": "github-release", "repository": "owner/repo",
            "asset_pattern": "Mod-*.zip", "extra": true
        }),
        json!({
            "kind": "git", "repository": "owner/repo", "branch": "main", "tag": "v1"
        }),
        json!({
            "kind": "nexus", "game_domain": "cyberpunk2077", "mod_id": 6945,
            "file_pattern": "Mod.zip", "file_id": 1
        }),
    ] {
        let error = format!("{:#}", parse(checkver.clone()).unwrap_err());
        assert!(error.contains("checkver"), "{checkver}: {error}");
    }
    for (checkver, field) in [
        (json!({"kind": "git", "repository": "owner/repo"}), "branch"),
        (
            json!({"kind": "github-release", "asset_pattern": "Mod-*.zip"}),
            "repository",
        ),
        (
            json!({"kind": "nexus", "game_domain": "cyberpunk2077", "file_pattern": "x.zip"}),
            "mod_id",
        ),
    ] {
        let error = format!("{:#}", parse(checkver).unwrap_err());
        assert!(error.contains(field), "missing {field}: {error}");
    }
}

#[test]
fn checkver_rejects_wrong_field_types() {
    for (checkver, field) in [
        (
            json!({
                "kind": "github-release", "repository": 1, "asset_pattern": "Mod-*.zip"
            }),
            "repository",
        ),
        (
            json!({
                "kind": "github-release", "repository": "owner/repo",
                "asset_pattern": true
            }),
            "asset_pattern",
        ),
        (
            json!({
                "kind": "github-release", "repository": "owner/repo",
                "asset_pattern": "Mod-*.zip", "prerelease": "yes"
            }),
            "prerelease",
        ),
        (
            json!({
                "kind": "nexus", "game_domain": "cyberpunk2077",
                "mod_id": "6945", "file_pattern": "Mod.zip"
            }),
            "mod_id",
        ),
    ] {
        let error = format!("{:#}", parse(checkver).unwrap_err());
        assert!(error.contains("checkver"), "{error}");
        assert!(error.contains(field), "{field}: {error}");
    }
}

#[test]
fn checkver_rejects_malformed_repositories() {
    for repository in [
        "",
        "owner",
        "/repo",
        "owner/",
        "owner/repo/extra",
        "owner/re po",
        "own er/repo",
        "owner/repo!",
        ".",
        "owner/.",
        "../repo",
        "owner/..",
        "üser/repo",
    ] {
        let error = format!(
            "{:#}",
            validate(json!({
                "kind": "git", "repository": repository, "branch": "main"
            }))
            .unwrap_err()
        );
        assert!(
            error.contains("package test-game/top") && error.contains("checkver.repository"),
            "{repository}: {error}"
        );
    }
}

#[test]
fn checkver_rejects_malformed_patterns() {
    for pattern in [
        String::new(),
        "a".repeat(201),
        "dir/Mod.zip".to_owned(),
        "dir\\Mod.zip".to_owned(),
        "Mod.zip\u{0}".to_owned(),
        "Mod\t.zip".to_owned(),
    ] {
        for (kind, field) in [
            ("asset_pattern", "asset_pattern"),
            ("file_pattern", "file_pattern"),
        ] {
            let checkver = if kind == "asset_pattern" {
                json!({
                    "kind": "github-release", "repository": "owner/repo",
                    "asset_pattern": pattern
                })
            } else {
                json!({
                    "kind": "nexus", "game_domain": "cyberpunk2077",
                    "mod_id": 6945, "file_pattern": pattern
                })
            };
            let error = format!("{:#}", validate(checkver).unwrap_err());
            assert!(
                error.contains("package test-game/top")
                    && error.contains(&format!("checkver.{field}")),
                "{pattern:?}: {error}"
            );
        }
    }
    validate(json!({
        "kind": "github-release", "repository": "owner/repo",
        "asset_pattern": &"a".repeat(200)
    }))
    .expect("200 characters are accepted");
}

#[test]
fn checkver_rejects_malformed_branches() {
    for branch in [
        "",
        "main branch",
        "main\tbranch",
        "main~1",
        "main^1",
        "main:1",
        "main?",
        "main*",
        "main[1]",
        r"main\1",
        "main..1",
        "main@{1}",
        "-main",
        "/main",
        "main/",
        "main.",
        "main.lock",
        &"a".repeat(256),
    ] {
        let error = format!(
            "{:#}",
            validate(json!({
                "kind": "git", "repository": "owner/repo", "branch": branch
            }))
            .unwrap_err()
        );
        assert!(
            error.contains("package test-game/top") && error.contains("checkver.branch"),
            "{branch:?}: {error}"
        );
    }
    for branch in ["main", "release/1.0", "feature-x", "a.b", &"a".repeat(255)] {
        validate(json!({
            "kind": "git", "repository": "owner/repo", "branch": branch
        }))
        .expect("valid branch");
    }
}

#[test]
fn checkver_rejects_malformed_nexus_domains_and_mod_ids() {
    for game_domain in [
        "",
        "Cyberpunk2077",
        "bad domain",
        "bad-",
        "-bad",
        &"a".repeat(65),
    ] {
        let error = format!(
            "{:#}",
            validate(json!({
                "kind": "nexus", "game_domain": game_domain,
                "mod_id": 6945, "file_pattern": "Mod.zip"
            }))
            .unwrap_err()
        );
        assert!(
            error.contains("package test-game/top") && error.contains("checkver.game_domain"),
            "{game_domain:?}: {error}"
        );
    }
    for mod_id in [0, 10_000_000_000u64] {
        let error = format!(
            "{:#}",
            validate(json!({
                "kind": "nexus", "game_domain": "cyberpunk2077",
                "mod_id": mod_id, "file_pattern": "Mod.zip"
            }))
            .unwrap_err()
        );
        assert!(
            error.contains("package test-game/top") && error.contains("checkver.mod_id"),
            "{mod_id}: {error}"
        );
    }
}
