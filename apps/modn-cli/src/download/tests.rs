use crate::commands::Cli;
use crate::download::{TEST_CALLS, TEST_RESPONSES};
use clap::Parser;
use modn_core::data_dir;
use modn_core::game_modules::with_module_source;
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;

const URL: &str = "https://github.com/fixture/project/releases/download/v1/example.zip";

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
