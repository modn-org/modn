use super::GithubReleaseAsset;
use crate::acquisition::{ArchiveCache, CacheKey, CacheUse};
use serde_json::json;
use std::io::Cursor;

fn selector() -> GithubReleaseAsset {
    GithubReleaseAsset {
        repository: "owner/project".into(),
        tag: "v1.2".into(),
        asset: "mod.zip".into(),
    }
}

fn response() -> serde_json::Value {
    json!({"tag_name":"v1.2", "html_url":"https://github.com/owner/project/releases/tag/v1.2",
        "assets":[{"name":"mod.zip", "browser_download_url":"https://github.com/owner/project/releases/download/v1.2/mod.zip"}]})
}

#[test]
fn resolves_pinned_asset_and_acquisition_checks_actual_bytes() {
    let source = selector();
    let url = source
        .resolve(|url| {
            assert_eq!(
                url,
                "https://api.github.com/repos/owner/project/releases/tags/v1.2"
            );
            Ok(Cursor::new(response().to_string()))
        })
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let cache = ArchiveCache::new(root.path().to_owned());
    let digest = blake3::hash(b"pinned bytes").to_hex().to_string();
    let key = CacheKey::new("game", "mod", "1", &url, &digest);
    assert!(cache
        .acquire(&key, CacheUse::Reuse, |_| Ok(Cursor::new(b"wrong bytes")))
        .is_err());
    let acquired = cache
        .acquire(&key, CacheUse::Reuse, |_| Ok(Cursor::new(b"pinned bytes")))
        .unwrap();
    assert_eq!(std::fs::read(acquired.path).unwrap(), b"pinned bytes");
}

#[test]
fn refuses_wrong_repository_tag_asset_duplicates_and_unsafe_urls() {
    let mut cases = Vec::new();
    let mut value = response();
    value["tag_name"] = json!("v2");
    cases.push(value);
    let mut value = response();
    value["html_url"] = json!("https://github.com/other/project/releases/tag/v1.2");
    cases.push(value);
    let mut value = response();
    value["assets"][0]["name"] = json!("other.zip");
    cases.push(value);
    let mut value = response();
    value["assets"] = json!([value["assets"][0], value["assets"][0]]);
    cases.push(value);
    for url in [
        "http://github.com/owner/project/releases/download/v1.2/mod.zip",
        "https://github.com@evil.invalid/mod.zip",
        "https://evil.invalid/mod.zip",
        "https://github.com/owner/project/releases/download/v1.2/../mod.zip",
    ] {
        let mut value = response();
        value["assets"][0]["browser_download_url"] = json!(url);
        cases.push(value);
    }
    for value in cases {
        assert!(
            selector()
                .resolve(|_| Ok(Cursor::new(value.to_string())))
                .is_err(),
            "{value}"
        );
    }
    for repository in ["../project", "owner/project/extra", "owner@evil/project"] {
        let mut source = selector();
        source.repository = repository.into();
        assert!(source.metadata_url().is_err());
    }
}
