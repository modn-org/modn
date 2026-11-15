use crate::commands::Cli;
use crate::download::{TEST_CALLS, TEST_METADATA_FAILURES, TEST_RESPONSES};
use clap::Parser;
use modn_core::acquisition::{CacheUse, FetchError, SourceAttempt, SourceTarget};
use modn_core::buckets::{BucketPackage, BucketStore};
use modn_core::data_dir;
use modn_core::game_modules::with_module_source;
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;

const URL: &str = "https://github.com/fixture/project/releases/download/v1/example.zip";
const METADATA_URL: &str = "https://api.github.com/repos/fixture/project/releases/tags/v1";

fn command(root: &Path, args: &[&str]) -> anyhow::Result<()> {
    let cli = Cli::try_parse_from(std::iter::once("modn").chain(args.iter().copied()))
        .expect("test command line");
    crate::run_cli(
        cli,
        &root.join("state.sqlite"),
        &crate::progress::Progress::start(None),
    )
}

fn archive(root: &Path) -> (Vec<u8>, String) {
    let path = root.join("example.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).expect("archive"));
    zip.start_file(
        "Demo/package/plugins/example.plugin",
        zip::write::SimpleFileOptions::default(),
    )
    .expect("entry");
    zip.write_all(b"cached-release").expect("payload");
    zip.finish().expect("finish");
    let bytes = fs::read(&path).expect("archive bytes");
    let digest = blake3::hash(&bytes).to_hex().to_string();
    (bytes, digest)
}

fn seed_bucket(root: &Path, digest: &str) {
    let bucket = root.join("buckets/main");
    let game = bucket.join("mods/test-game");
    fs::create_dir_all(&game).expect("bucket directory");
    fs::write(
        bucket.join("bucket.json"),
        br#"{"schema_version":2,"name":"main","title":"Fixture"}"#,
    )
    .expect("bucket metadata");
    fs::write(
        game.join("example.json"),
        serde_json::to_vec(&json!({
            "schema_version": 2,
            "id": "example",
            "game": "test-game",
            "name": "Example",
            "version": "1.0",
            "archive_format": "zip",
            "blake3": digest,
            "sources": [{
                "kind": "github-release",
                "repository": "fixture/project",
                "tag": "v1",
                "asset": "example.zip",
            }],
        }))
        .expect("manifest JSON"),
    )
    .expect("manifest");
}

fn seed_cache(root: &Path, bytes: &[u8]) {
    let cache = root.join("cache");
    fs::create_dir_all(&cache).expect("cache directory");
    let hash = blake3::hash(URL.as_bytes()).to_hex().to_string();
    fs::write(
        cache.join(format!("test-game#example#1.0#{}.zip", &hash[..7])),
        bytes,
    )
    .expect("cache entry");
}

#[test]
fn undo_restores_a_github_release_from_the_cache_without_network() {
    let temporary = tempfile::tempdir().expect("fixture");
    let root = temporary.path().join("manager");
    fs::create_dir_all(&root).expect("manager directory");
    let (bytes, digest) = archive(temporary.path());
    seed_bucket(&root, &digest);
    seed_cache(&root, &bytes);
    let game = temporary.path().join("game");
    fs::create_dir_all(game.join("bin")).expect("game directory");
    fs::write(game.join("bin/TestGame.exe"), b"game-exe").expect("marker");
    let module = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/test-game");
    let game_arg = game.to_str().expect("game path").to_owned();

    data_dir::with_data_directory(&root, || {
        with_module_source(Some(&module), || {
            command(&root, &["game", "add", "test-game", "--path", &game_arg])
                .expect("register game");
            command(
                &root,
                &["--json", "install", "example", "--path", &game_arg, "--yes"],
            )
            .expect("install from the seeded cache");
            let plugin = game.join("addons/example.plugin");
            assert_eq!(
                fs::read(&plugin).expect("installed plugin"),
                b"cached-release"
            );
            command(&root, &["--json", "uninstall", "example", "--yes"]).expect("uninstall");
            assert!(!plugin.exists());

            // No fixture response matches the metadata or asset URL, so any
            // network request fails instead of reaching GitHub.
            TEST_RESPONSES.with(|responses| responses.replace(Some(BTreeMap::new())));
            TEST_CALLS.with(|calls| calls.borrow_mut().clear());
            command(&root, &["--json", "undo", "--yes"]).expect("undo from the cache");
            assert!(
                TEST_CALLS.with(|calls| calls.borrow().is_empty()),
                "undo contacted the provider"
            );
            assert_eq!(
                fs::read(&plugin).expect("restored plugin"),
                b"cached-release"
            );
        });
    });
    TEST_RESPONSES.with(|responses| responses.replace(None));
}

#[test]
fn undo_downloads_a_github_release_again_after_a_cache_miss() {
    let temporary = tempfile::tempdir().expect("fixture");
    let root = temporary.path().join("manager");
    fs::create_dir_all(&root).expect("manager directory");
    let (bytes, digest) = archive(temporary.path());
    seed_bucket(&root, &digest);
    seed_cache(&root, &bytes);
    let game = temporary.path().join("game");
    fs::create_dir_all(game.join("bin")).expect("game directory");
    fs::write(game.join("bin/TestGame.exe"), b"game-exe").expect("marker");
    let module = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/test-game");
    let game_arg = game.to_str().expect("game path").to_owned();

    data_dir::with_data_directory(&root, || {
        with_module_source(Some(&module), || {
            command(&root, &["game", "add", "test-game", "--path", &game_arg])
                .expect("register game");
            command(
                &root,
                &["--json", "install", "example", "--path", &game_arg, "--yes"],
            )
            .expect("install from the seeded cache");
            let plugin = game.join("addons/example.plugin");
            assert_eq!(
                fs::read(&plugin).expect("installed plugin"),
                b"cached-release"
            );
            command(&root, &["--json", "uninstall", "example", "--yes"]).expect("uninstall");
            assert!(!plugin.exists());
            fs::remove_dir_all(root.join("cache")).expect("clear the archive cache");

            let metadata = json!({
                "tag_name": "v1",
                "html_url": "https://github.com/fixture/project/releases/tag/v1",
                "assets": [{"name": "example.zip", "browser_download_url": URL}],
            });
            TEST_RESPONSES.with(|responses| {
                responses.replace(Some(BTreeMap::from([
                    (
                        METADATA_URL.to_owned(),
                        serde_json::to_vec(&metadata).expect("metadata JSON"),
                    ),
                    (URL.to_owned(), bytes.clone()),
                ])))
            });
            TEST_CALLS.with(|calls| calls.borrow_mut().clear());
            command(&root, &["--json", "undo", "--yes"]).expect("undo downloads again");

            let calls = TEST_CALLS.with(|calls| calls.borrow().clone());
            assert_eq!(
                calls,
                vec![METADATA_URL.to_owned(), URL.to_owned()],
                "undo did not resolve and download the recorded source"
            );
            assert_eq!(
                fs::read(&plugin).expect("restored plugin"),
                b"cached-release"
            );
            let cached: Vec<_> = fs::read_dir(root.join("cache"))
                .expect("cache directory")
                .map(|entry| entry.expect("cache entry").path())
                .filter(|path| path.extension().is_some_and(|extension| extension == "zip"))
                .collect();
            assert_eq!(cached.len(), 1, "undo did not acquire the archive again");
        });
    });
    TEST_RESPONSES.with(|responses| responses.replace(None));
}

const FALLBACK_URL: &str = "https://fixture.invalid/example.zip";

fn package_with_sources(root: &Path, digest: &str, sources: serde_json::Value) -> BucketPackage {
    let bucket = root.join("buckets/main");
    let game = bucket.join("mods/test-game");
    fs::create_dir_all(&game).expect("bucket directory");
    fs::write(
        bucket.join("bucket.json"),
        br#"{"schema_version":2,"name":"main","title":"Fixture"}"#,
    )
    .expect("bucket metadata");
    fs::write(
        game.join("example.json"),
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

fn acquire_with_sources(
    root: &Path,
    package: &BucketPackage,
) -> anyhow::Result<(Vec<SourceAttempt>, std::path::PathBuf)> {
    let cache = crate::download::cache_for(&root.join("state.sqlite")).expect("cache");
    let mut archives = BTreeMap::new();
    let mut digests = BTreeMap::new();
    let attempts = super::download(
        &cache,
        CacheUse::Refresh,
        &[package],
        &mut archives,
        &mut digests,
        &[],
        true,
        &crate::progress::Progress::start(None),
    )?;
    assert_eq!(archives.len(), 1, "the acquired archive was not recorded");
    let archive = archives.into_values().next().expect("acquired archive");
    Ok((attempts, archive))
}

fn github_package(root: &Path, digest: &str) -> BucketPackage {
    package_with_sources(
        root,
        digest,
        json!([{
            "kind": "github-release",
            "repository": "fixture/project",
            "tag": "v1",
            "asset": "example.zip"
        }]),
    )
}

fn github_metadata() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "tag_name": "v1",
        "html_url": "https://github.com/fixture/project/releases/tag/v1",
        "assets": [{"name": "example.zip", "browser_download_url": URL}],
    }))
    .expect("metadata JSON")
}

#[test]
fn a_transient_metadata_failure_is_retried_before_the_source_is_refused() {
    let temporary = tempfile::tempdir().expect("fixture");
    let root = temporary.path().join("manager");
    fs::create_dir_all(&root).expect("manager directory");
    let (bytes, digest) = archive(temporary.path());
    let package = github_package(&root, &digest);
    TEST_RESPONSES.with(|responses| {
        responses.replace(Some(BTreeMap::from([
            (METADATA_URL.to_owned(), github_metadata()),
            (URL.to_owned(), bytes.clone()),
        ])))
    });
    TEST_CALLS.with(|calls| calls.borrow_mut().clear());
    TEST_METADATA_FAILURES.with(|failures| {
        failures.replace(Some(BTreeMap::from([(
            METADATA_URL.to_owned(),
            vec![FetchError::Timeout],
        )])))
    });
    let candidates = super::fallback::candidates(&package);
    TEST_RESPONSES.with(|responses| responses.replace(None));
    TEST_METADATA_FAILURES.with(|failures| failures.replace(None));

    assert!(
        matches!(candidates[0].target, SourceTarget::Fetch(_)),
        "a transient metadata failure refused the source instead of retrying: {candidates:?}"
    );
    let calls = TEST_CALLS.with(|calls| calls.borrow().clone());
    assert_eq!(
        calls.iter().filter(|url| *url == METADATA_URL).count(),
        2,
        "the metadata request was not retried: {calls:?}"
    );
}

#[test]
fn a_404_metadata_failure_is_refused_without_a_retry() {
    let temporary = tempfile::tempdir().expect("fixture");
    let root = temporary.path().join("manager");
    fs::create_dir_all(&root).expect("manager directory");
    let (bytes, digest) = archive(temporary.path());
    let package = github_package(&root, &digest);
    TEST_RESPONSES.with(|responses| {
        responses.replace(Some(BTreeMap::from([
            (METADATA_URL.to_owned(), github_metadata()),
            (URL.to_owned(), bytes.clone()),
        ])))
    });
    TEST_CALLS.with(|calls| calls.borrow_mut().clear());
    TEST_METADATA_FAILURES.with(|failures| {
        failures.replace(Some(BTreeMap::from([(
            METADATA_URL.to_owned(),
            vec![FetchError::Http(404)],
        )])))
    });
    let candidates = super::fallback::candidates(&package);
    TEST_RESPONSES.with(|responses| responses.replace(None));
    TEST_METADATA_FAILURES.with(|failures| failures.replace(None));

    assert!(
        matches!(candidates[0].target, SourceTarget::Refused(_)),
        "a 404 metadata response was not refused: {candidates:?}"
    );
    let calls = TEST_CALLS.with(|calls| calls.borrow().clone());
    assert_eq!(
        calls.iter().filter(|url| *url == METADATA_URL).count(),
        1,
        "a 404 metadata response was retried: {calls:?}"
    );
}

#[test]
fn a_failed_nexus_primary_falls_back_to_the_https_source() {
    let temporary = tempfile::tempdir().expect("fixture");
    let root = temporary.path().join("manager");
    fs::create_dir_all(&root).expect("manager directory");
    let (bytes, digest) = archive(temporary.path());
    let package = package_with_sources(
        &root,
        &digest,
        json!([
            {
                "kind": "nexus",
                "game_domain": "cyberpunk2077",
                "mod_id": 6945,
                "file_id": 123456,
                "file_name": "NexusMod.zip"
            },
            {"kind": "https", "url": FALLBACK_URL}
        ]),
    );
    TEST_RESPONSES.with(|responses| {
        responses.replace(Some(BTreeMap::from([(
            FALLBACK_URL.to_owned(),
            bytes.clone(),
        )])))
    });
    let result = crate::nexus_auth::with_test_key(crate::nexus_auth::TestKey::Missing, || {
        acquire_with_sources(&root, &package)
    });
    let attempts = result.expect("the https source supplies the archive");
    TEST_RESPONSES.with(|responses| responses.replace(None));
    let (attempts, archive) = attempts;
    assert!(
        attempts.len() >= 2,
        "the Nexus attempt was not recorded: {attempts:?}"
    );
    assert!(
        attempts[0].source.contains("Nexus Mods"),
        "the first attempt is not the Nexus source: {attempts:?}"
    );
    assert!(
        attempts[0]
            .failure
            .as_ref()
            .is_some_and(|failure| failure.to_string().contains(crate::nexus_auth::MISSING_KEY)),
        "the Nexus failure is not named: {attempts:?}"
    );
    assert!(
        attempts.last().expect("last attempt").succeeded(),
        "the https fallback did not succeed: {attempts:?}"
    );
    assert_eq!(
        fs::read(archive).expect("fallback archive bytes"),
        bytes,
        "the fallback archive does not hold the https bytes"
    );
}

#[test]
fn a_lone_nexus_source_still_fails_with_the_missing_key_message() {
    let temporary = tempfile::tempdir().expect("fixture");
    let root = temporary.path().join("manager");
    fs::create_dir_all(&root).expect("manager directory");
    let (_, digest) = archive(temporary.path());
    let package = package_with_sources(
        &root,
        &digest,
        json!([{
            "kind": "nexus",
            "game_domain": "cyberpunk2077",
            "mod_id": 6945,
            "file_id": 123456,
            "file_name": "NexusMod.zip"
        }]),
    );
    let error = crate::nexus_auth::with_test_key(crate::nexus_auth::TestKey::Missing, || {
        acquire_with_sources(&root, &package)
    })
    .expect_err("a lone Nexus source still fails");
    let message = format!("{error:#}");
    assert!(
        message.contains(crate::nexus_auth::MISSING_KEY),
        "the clear missing-key message is gone: {message}"
    );
    assert!(!message.contains("fallback"));
}
