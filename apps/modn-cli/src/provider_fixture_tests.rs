use crate::commands::Cli;
use crate::download::{TEST_CALLS, TEST_RESPONSES};
use clap::Parser;
use modn_core::data_dir;
use modn_core::game_modules::with_module_source;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;

#[test]
fn provider_commands_fetch_only_after_consent_and_reject_changed_evidence() {
    let fixture = tempfile::tempdir().unwrap();
    let manager = fixture.path().join("manager");
    let root = manager.as_path();
    fs::create_dir_all(root.join("buckets/main/games")).unwrap();
    fs::write(
        root.join("buckets/main/bucket.json"),
        r#"{"schema_version":1,"name":"main","title":"Fixture"}"#,
    )
    .unwrap();
    let game = fixture.path().join("game");
    fs::create_dir_all(game.join("bin")).unwrap();
    fs::write(game.join("bin/TestGame.exe"), b"synthetic marker").unwrap();
    let archive = root.join("mod.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
    zip.start_file(
        "Demo/package/plugins/provider.plugin",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(b"verified fixture").unwrap();
    zip.finish().unwrap();
    let bytes = fs::read(archive).unwrap();
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let catalog = root.join("catalog");
    fs::create_dir_all(catalog.join("games/test-game")).unwrap();
    fs::write(
        catalog.join("bucket.json"),
        r#"{"schema_version":1,"name":"fixture","title":"Fixture"}"#,
    )
    .unwrap();
    let manifest = catalog.join("games/test-game/provider.json");
    let mut package = json!({"schema_version":1,"game":"test-game","id":"provider","name":"Provider","version":"1","archive_format":"zip","sha256":hash,
        "source":{"kind":"github-release","repository":"fixture/project","tag":"v1","asset":"mod.zip"}});
    fs::write(&manifest, serde_json::to_vec(&package).unwrap()).unwrap();
    let metadata_url = "https://api.github.com/repos/fixture/project/releases/tags/v1";
    let asset_url = "https://github.com/fixture/project/releases/download/v1/mod.zip";
    let metadata = json!({"tag_name":"v1","html_url":"https://github.com/fixture/project/releases/tag/v1", "assets":[{"name":"mod.zip","browser_download_url":asset_url}]});
    TEST_RESPONSES.with(|responses| {
        responses.replace(Some(BTreeMap::from([
            (
                metadata_url.to_owned(),
                serde_json::to_vec(&metadata).unwrap(),
            ),
            (asset_url.to_owned(), bytes),
        ])))
    });
    TEST_CALLS.with(|calls| calls.borrow_mut().clear());
    let module = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/test-game");
    data_dir::with_data_directory(root, || {
        with_module_source(Some(&module), || {
            let command = |args: &[&str]| {
                let cli = Cli::try_parse_from(std::iter::once("modn").chain(args.iter().copied()))
                    .unwrap();
                crate::run_cli(cli, &root.join("state.sqlite"))
            };
            command(&["bucket", "add", "fixture", catalog.to_str().unwrap()]).unwrap();
            let preview = [
                "--json",
                "install",
                "provider",
                "--path",
                game.to_str().unwrap(),
                "--dry-run",
            ];
            command(&preview).unwrap();
            assert!(!root.join("state.sqlite").exists());
            assert!(!root.join("cache").exists());
            assert!(TEST_CALLS.with(|calls| calls.borrow().is_empty()));
            assert!(command(&["install", "provider", "--path", game.to_str().unwrap()]).is_err());
            assert!(TEST_CALLS.with(|calls| calls.borrow().is_empty()));
            for invalid in [
                json!({"tag_name":"wrong","html_url":"https://github.com/fixture/project/releases/tag/v1","assets":[]}),
                json!({"tag_name":"v1","html_url":"https://github.com/fixture/project/releases/tag/v1","assets":[{"name":"mod.zip","browser_download_url":"https://evil.invalid/mod.zip"}]}),
            ] {
                TEST_RESPONSES.with(|responses| {
                    responses.borrow_mut().as_mut().unwrap().insert(
                        metadata_url.to_owned(),
                        serde_json::to_vec(&invalid).unwrap(),
                    )
                });
                assert!(command(&[
                    "install",
                    "provider",
                    "--path",
                    game.to_str().unwrap(),
                    "--yes"
                ])
                .is_err());
                assert!(!game.join("addons").exists());
            }
            TEST_RESPONSES.with(|responses| {
                responses.borrow_mut().as_mut().unwrap().insert(
                    metadata_url.to_owned(),
                    serde_json::to_vec(&metadata).unwrap(),
                )
            });
            package["sha256"] = json!("0".repeat(64));
            fs::write(&manifest, serde_json::to_vec(&package).unwrap()).unwrap();
            assert!(command(&[
                "install",
                "provider",
                "--path",
                game.to_str().unwrap(),
                "--yes"
            ])
            .is_err());
            assert!(!game.join("addons").exists());
            package["sha256"] = json!(hash);
            fs::write(&manifest, serde_json::to_vec(&package).unwrap()).unwrap();
            command(&[
                "install",
                "provider",
                "--path",
                game.to_str().unwrap(),
                "--yes",
            ])
            .unwrap();
            assert_eq!(
                fs::read(game.join("addons/provider.plugin")).unwrap(),
                b"verified fixture"
            );
            assert!(TEST_CALLS.with(|calls| calls.borrow().iter().any(|url| url == asset_url)));
        })
    });
    TEST_RESPONSES.with(|responses| responses.replace(None));
}
