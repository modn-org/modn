use super::{plan_sources, source_cache_url, PlannedAction};
use crate::acquisition::{GithubReleaseAsset, SourceCandidate};
use crate::buckets::{BucketPackage, PackageManifest, PackageSource};

const HTTPS_URL: &str = "https://example.org/mod.zip";
const GITHUB_URL: &str = "https://github.com/fixture/project/releases/download/v1/example.zip";
const NEXUS_URL: &str = "nexus://cyberpunk2077/mods/6945/files/123456/NexusMod.zip";
const GIT_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

fn fixture_release(sources: serde_json::Value) -> BucketPackage {
    let package: PackageManifest = serde_json::from_value(serde_json::json!({
        "schema_version": 2,
        "id": "example",
        "game": "test-game",
        "name": "Example",
        "version": "1.0",
        "archive_format": "zip",
        "blake3": "0".repeat(64),
        "sources": sources,
    }))
    .expect("fixture manifest");
    BucketPackage {
        bucket: "main".to_owned(),
        package,
    }
}

fn github_source() -> PackageSource {
    PackageSource::GithubRelease {
        repository: "fixture/project".to_owned(),
        tag: "v1".to_owned(),
        asset: "example.zip".to_owned(),
    }
}

fn nexus_source() -> PackageSource {
    PackageSource::Nexus {
        game_domain: "cyberpunk2077".to_owned(),
        mod_id: 6945,
        file_id: 123456,
        file_name: "NexusMod.zip".to_owned(),
    }
}

#[test]
fn an_https_source_keeps_its_url_as_the_cache_identity() {
    let source = PackageSource::Https {
        url: HTTPS_URL.to_owned(),
    };
    assert_eq!(
        source_cache_url(&source).expect("a valid HTTPS source"),
        Some(HTTPS_URL.to_owned())
    );
}

#[test]
fn a_github_release_source_uses_the_canonical_asset_url() {
    assert_eq!(
        source_cache_url(&github_source()).expect("a valid GitHub release source"),
        Some(GITHUB_URL.to_owned())
    );
}

#[test]
fn a_nexus_source_uses_the_credential_free_file_url() {
    assert_eq!(
        source_cache_url(&nexus_source()).expect("a valid Nexus source"),
        Some(NEXUS_URL.to_owned())
    );
}

#[test]
fn git_and_manual_sources_have_no_archive_cache_identity() {
    for source in [
        PackageSource::Git {
            repository: "fixture/project".to_owned(),
            commit: GIT_COMMIT.to_owned(),
        },
        PackageSource::Manual {
            filename: "example.zip".to_owned(),
        },
    ] {
        assert_eq!(
            source_cache_url(&source).expect("a source without a selector error"),
            None,
            "{source:?} must not name an archive cache entry"
        );
    }
}

#[test]
fn an_invalid_nexus_selector_reports_an_error() {
    let source = PackageSource::Nexus {
        game_domain: "Cyberpunk2077".to_owned(),
        mod_id: 6945,
        file_id: 123456,
        file_name: "NexusMod.zip".to_owned(),
    };
    assert!(
        source_cache_url(&source).is_err(),
        "an invalid Nexus game domain must not produce a cache URL"
    );
}

#[test]
fn a_release_plans_its_sources_in_manifest_order() {
    let release = fixture_release(serde_json::json!([
        {
            "kind": "nexus",
            "game_domain": "cyberpunk2077",
            "mod_id": 6945,
            "file_id": 123456,
            "file_name": "NexusMod.zip"
        },
        {"kind": "https", "url": HTTPS_URL},
        {
            "kind": "github-release",
            "repository": "fixture/project",
            "tag": "v1",
            "asset": "example.zip"
        },
        {"kind": "manual", "filename": "example.zip"}
    ]));
    let planned = plan_sources(&release);

    assert_eq!(planned.len(), 4, "the plan dropped a source");
    assert_eq!(
        planned[0].action,
        PlannedAction::Nexus {
            game_domain: "cyberpunk2077".to_owned(),
            mod_id: 6945,
            file_id: 123456,
            file_name: "NexusMod.zip".to_owned(),
        }
    );
    assert_eq!(
        planned[1].action,
        PlannedAction::Fetch {
            candidate: SourceCandidate::fetch(HTTPS_URL),
        }
    );
    assert_eq!(
        planned[2].action,
        PlannedAction::Github {
            selector: GithubReleaseAsset::new(
                "fixture/project".to_owned(),
                "v1".to_owned(),
                "example.zip".to_owned(),
            ),
            expected: GITHUB_URL.to_owned(),
        }
    );
    assert_eq!(
        planned[3].action,
        PlannedAction::Fetch {
            candidate: SourceCandidate::manual("example.zip"),
        }
    );

    let identities: Vec<Option<String>> = planned
        .iter()
        .map(|step| source_cache_url(&step.source).expect("a valid fixture source"))
        .collect();
    assert_eq!(
        identities,
        vec![
            Some(NEXUS_URL.to_owned()),
            Some(HTTPS_URL.to_owned()),
            Some(GITHUB_URL.to_owned()),
            None,
        ],
        "the plan changed the order or the cache identity of a source"
    );
}

#[test]
fn a_git_release_plans_its_pinned_commit_as_one_source() {
    let release = fixture_release(serde_json::json!([
        {"kind": "git", "repository": "fixture/project", "commit": GIT_COMMIT}
    ]));
    let planned = plan_sources(&release);

    assert_eq!(planned.len(), 1, "the plan dropped the Git source");
    assert_eq!(
        planned[0].action,
        PlannedAction::Git {
            repository: "fixture/project".to_owned(),
            commit: GIT_COMMIT.to_owned(),
        }
    );
    assert_eq!(
        source_cache_url(&planned[0].source).expect("a valid fixture source"),
        None,
        "a Git source is keyed by its commit, not by an archive URL"
    );
}
