//! Cancellation and text-reporting tests for ordered archive acquisition.

use super::download_into;
use crate::download::{TEST_CALLS, TEST_RESPONSES};
use modn_core::acquisition::{CacheUse, NexusApiKey, SourceAttempt};
use modn_core::buckets::{BucketPackage, BucketStore};
use modn_core::data_dir;
use modn_core::error::{code_of, ErrorCode};
use modn_core::progress::{CancellationToken, OperationControl};
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const FIRST: &str = "https://fixture.invalid/primary.zip";
const SECOND: &str = "https://fixture.invalid/mirror.zip";
const API_KEY: &str = "SECRETAPIKEY123";
const CDN: &str = "https://nexus-cdn.com/files/NexusMod.zip";
const API_URL: &str =
    "https://api.nexusmods.com/v1/games/cyberpunk2077/mods/6945/files/123456/download_link.json";

fn archive(root: &Path) -> (Vec<u8>, String) {
    let path = root.join("example.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).expect("archive"));
    zip.start_file(
        "Demo/package/plugins/example.plugin",
        zip::write::SimpleFileOptions::default(),
    )
    .expect("entry");
    zip.write_all(b"release-payload").expect("payload");
    zip.finish().expect("finish");
    let bytes = fs::read(&path).expect("archive bytes");
    let digest = blake3::hash(&bytes).to_hex().to_string();
    (bytes, digest)
}

fn package(root: &Path, digest: &str, sources: serde_json::Value) -> BucketPackage {
    let bucket = root.join("buckets/main");
    let records = bucket.join("mods/test-game");
    fs::create_dir_all(&records).expect("bucket directory");
    fs::write(
        bucket.join("bucket.json"),
        br#"{"schema_version":2,"name":"main","title":"Fixture"}"#,
    )
    .expect("bucket metadata");
    fs::write(
        records.join("example.json"),
        serde_json::to_vec(&json!({
            "schema_version": 2,
            "id": "example",
            "game": "test-game",
            "name": "Example",
            "version": "1.0",
            "archive_format": "zip",
            "blake3": digest,
            "sources": sources,
        }))
        .expect("manifest JSON"),
    )
    .expect("manifest");
    data_dir::with_data_directory(root, || BucketStore::new(root).packages(Some("test-game")))
        .expect("packages")
        .into_iter()
        .next()
        .expect("example package")
}

fn nexus_then_https(root: &Path, digest: &str) -> BucketPackage {
    package(
        root,
        digest,
        json!([
            {
                "kind": "nexus",
                "game_domain": "cyberpunk2077",
                "mod_id": 6945,
                "file_id": 123456,
                "file_name": "NexusMod.zip"
            },
            {"kind": "https", "url": FIRST}
        ]),
    )
}

/// Seed the cache entry an https source would supply, keyed by its URL.
fn seed_cache(root: &Path, url: &str, bytes: &[u8]) {
    let cache = root.join("cache");
    fs::create_dir_all(&cache).expect("cache directory");
    let hash = blake3::hash(url.as_bytes()).to_hex().to_string();
    fs::write(
        cache.join(format!("test-game#example#1.0#{}.zip", &hash[..7])),
        bytes,
    )
    .expect("cache entry");
}

fn cancelled_control() -> OperationControl {
    let token = CancellationToken::new();
    token.cancel();
    OperationControl::new(token, |_event| {})
}

fn acquire(
    root: &Path,
    package: &BucketPackage,
    cache_use: CacheUse,
    control: &OperationControl,
    text: bool,
) -> (anyhow::Result<Vec<SourceAttempt>>, String) {
    let cache = crate::download::cache_for(&root.join("state.sqlite")).expect("cache");
    let mut archives = BTreeMap::new();
    let mut digests = BTreeMap::new();
    let mut report = Vec::new();
    let result = download_into(
        &cache,
        cache_use,
        &[package],
        &mut archives,
        &mut digests,
        &[],
        text,
        &mut report,
        control,
    );
    (result, String::from_utf8(report).expect("UTF-8 report"))
}

fn partial_files(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(root.join("cache")) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "download")
        })
        .collect()
}

fn with_nexus_key<T>(run: impl FnOnce() -> T) -> T {
    let key = NexusApiKey::new(API_KEY).expect("valid fixture key");
    crate::nexus_auth::with_test_key(crate::nexus_auth::TestKey::Present(key), run)
}

fn nexus_responses(body: &[u8]) {
    let api = serde_json::to_vec(&json!([{"URI": CDN}])).expect("JSON");
    super::super::nexus_download::set_responses(BTreeMap::from([
        (
            API_URL.to_owned(),
            super::super::nexus_download::FixtureResponse::ok(api),
        ),
        (
            CDN.to_owned(),
            super::super::nexus_download::FixtureResponse::ok(body.to_vec()),
        ),
    ]));
}

#[test]
fn a_cancel_during_a_nexus_attempt_stops_before_the_cached_https_fallback() {
    let temporary = tempfile::tempdir().expect("fixture");
    let root = temporary.path().join("manager");
    fs::create_dir_all(&root).expect("manager directory");
    let (bytes, digest) = archive(temporary.path());
    let package = nexus_then_https(&root, &digest);
    seed_cache(&root, FIRST, &bytes);
    nexus_responses(&bytes);
    TEST_CALLS.with(|calls| calls.borrow_mut().clear());
    TEST_RESPONSES.with(|responses| responses.replace(Some(BTreeMap::new())));
    let control = cancelled_control();

    let (result, text) =
        with_nexus_key(|| acquire(&root, &package, CacheUse::Reuse, &control, false));

    TEST_RESPONSES.with(|responses| responses.replace(None));
    super::super::nexus_download::set_responses(BTreeMap::new());
    let error = result.expect_err("a cancelled Nexus attempt stops the whole acquisition");
    assert_eq!(
        code_of(&error),
        Some(ErrorCode::Cancelled),
        "the cancellation was not preserved: {error:#} (report: {text})"
    );
    let calls = TEST_CALLS.with(|calls| calls.borrow().clone());
    assert!(
        !calls.iter().any(|url| url == FIRST),
        "the acquisition fell back to the https source after a cancellation: {calls:?}"
    );
    assert!(
        partial_files(&root).is_empty(),
        "a cancelled Nexus attempt kept a partial file"
    );
}

#[test]
fn a_cancel_before_an_https_attempt_stops_before_the_second_source() {
    let temporary = tempfile::tempdir().expect("fixture");
    let root = temporary.path().join("manager");
    fs::create_dir_all(&root).expect("manager directory");
    let (bytes, digest) = archive(temporary.path());
    let package = package(
        &root,
        &digest,
        json!([
            {"kind": "https", "url": FIRST},
            {"kind": "https", "url": SECOND}
        ]),
    );
    seed_cache(&root, SECOND, &bytes);
    TEST_CALLS.with(|calls| calls.borrow_mut().clear());
    TEST_RESPONSES.with(|responses| responses.replace(Some(BTreeMap::new())));
    let control = cancelled_control();

    let (result, _) = acquire(&root, &package, CacheUse::Reuse, &control, false);

    TEST_RESPONSES.with(|responses| responses.replace(None));
    let error = result.expect_err("a cancelled https attempt stops the whole acquisition");
    assert_eq!(
        code_of(&error),
        Some(ErrorCode::Cancelled),
        "the cancellation was not preserved: {error:#}"
    );
    let calls = TEST_CALLS.with(|calls| calls.borrow().clone());
    assert!(
        !calls.iter().any(|url| url == FIRST || url == SECOND),
        "a cancelled acquisition opened a later source: {calls:?}"
    );
}

#[test]
fn a_failed_nexus_attempt_is_reported_before_the_serving_https_source() {
    let temporary = tempfile::tempdir().expect("fixture");
    let root = temporary.path().join("manager");
    fs::create_dir_all(&root).expect("manager directory");
    let (bytes, digest) = archive(temporary.path());
    let package = nexus_then_https(&root, &digest);
    TEST_RESPONSES.with(|responses| {
        responses.replace(Some(BTreeMap::from([(FIRST.to_owned(), bytes.clone())])))
    });
    let control = OperationControl::default();

    let (result, text) =
        crate::nexus_auth::with_test_key(crate::nexus_auth::TestKey::Missing, || {
            acquire(&root, &package, CacheUse::Refresh, &control, true)
        });

    TEST_RESPONSES.with(|responses| responses.replace(None));
    result.expect("the https source supplies the archive");
    assert!(
        text.contains("Nexus Mods: cyberpunk2077 mod 6945 file 123456 (NexusMod.zip)"),
        "the text report omits the Nexus attempt: {text}"
    );
    assert!(
        text.contains(crate::nexus_auth::MISSING_KEY),
        "the text report omits the Nexus failure reason: {text}"
    );
    assert!(
        text.contains(FIRST) && text.contains("ok"),
        "the text report omits the serving https source: {text}"
    );
    assert!(!text.contains(API_KEY), "the text report leaks the API key");
}

const METADATA: &str = "https://api.github.com/repos/fixture/project/releases/tags/v1";
const ASSET: &str = "https://github.com/fixture/project/releases/download/v1/example.zip";

fn https_then_github(root: &Path, digest: &str) -> BucketPackage {
    package(
        root,
        digest,
        json!([
            {"kind": "https", "url": FIRST},
            {
                "kind": "github-release",
                "repository": "fixture/project",
                "tag": "v1",
                "asset": "example.zip"
            }
        ]),
    )
}

fn github_metadata() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "tag_name": "v1",
        "html_url": "https://github.com/fixture/project/releases/tag/v1",
        "assets": [{"name": "example.zip", "browser_download_url": ASSET}],
    }))
    .expect("metadata JSON")
}

#[test]
fn a_succeeding_primary_makes_no_metadata_request_for_a_github_fallback() {
    let temporary = tempfile::tempdir().expect("fixture");
    let root = temporary.path().join("manager");
    fs::create_dir_all(&root).expect("manager directory");
    let (bytes, digest) = archive(temporary.path());
    let package = https_then_github(&root, &digest);
    TEST_RESPONSES
        .with(|responses| responses.replace(Some(BTreeMap::from([(FIRST.to_owned(), bytes)]))));
    TEST_CALLS.with(|calls| calls.borrow_mut().clear());
    let control = OperationControl::default();

    let (result, _) = acquire(&root, &package, CacheUse::Refresh, &control, false);

    let calls = TEST_CALLS.with(|calls| calls.borrow().clone());
    TEST_RESPONSES.with(|responses| responses.replace(None));
    result.expect("the primary https source supplies the archive");
    assert!(
        !calls.iter().any(|url| url == METADATA),
        "an unused GitHub fallback fetched provider metadata: {calls:?}"
    );
}
