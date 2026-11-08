use crate::commands::Cli;
use crate::download::nexus_download::{self, FixtureResponse, RecordedRequest};
use crate::nexus_auth::{self, TestKey};
use clap::Parser;
use modn_core::acquisition::NexusApiKey;
use modn_core::data_dir;
use modn_core::game_modules::with_module_source;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const API_KEY: &str = "SECRETAPIKEY123";
const NXM_KEY: &str = "SECRETNXMKEY456";
const CDN_TOKEN: &str = "SECRETCDN789";
const SECRETS: [&str; 3] = [API_KEY, NXM_KEY, CDN_TOKEN];
const DOMAIN: &str = "cyberpunk2077";
const MOD_ID: u64 = 6945;
const FILE_ID: u64 = 123456;
const CDN_URL: &str = "https://nexus-cdn.com/files/NexusMod.zip?token=SECRETCDN789";

fn api_url(file_id: u64, grant: Option<u64>) -> String {
    let base = format!(
        "https://api.nexusmods.com/v1/games/{DOMAIN}/mods/{MOD_ID}/files/{file_id}/download_link.json"
    );
    match grant {
        Some(expires) => format!("{base}?key={NXM_KEY}&expires={expires}"),
        None => base,
    }
}

fn expires() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after the Unix epoch")
        .as_secs()
        + 3600
}

fn nxm_link(file_id: u64, expires: u64) -> String {
    format!("nxm://{DOMAIN}/mods/{MOD_ID}/files/{file_id}?key={NXM_KEY}&expires={expires}")
}

fn response_map(
    entries: impl IntoIterator<Item = (String, FixtureResponse)>,
) -> BTreeMap<String, FixtureResponse> {
    entries.into_iter().collect()
}

fn api_link_body(cdn: &str) -> Vec<u8> {
    serde_json::to_vec(&json!([{ "URI": cdn }])).expect("JSON")
}

fn free_responses(
    file_id: u64,
    expires: u64,
    cdn: &str,
    body: &[u8],
) -> BTreeMap<String, FixtureResponse> {
    response_map([
        (
            api_url(file_id, Some(expires)),
            FixtureResponse::ok(api_link_body(cdn)),
        ),
        (cdn.to_owned(), FixtureResponse::ok(body.to_vec())),
    ])
}

fn premium_responses(file_id: u64, cdn: &str, body: &[u8]) -> BTreeMap<String, FixtureResponse> {
    response_map([
        (
            api_url(file_id, None),
            FixtureResponse::ok(api_link_body(cdn)),
        ),
        (cdn.to_owned(), FixtureResponse::ok(body.to_vec())),
    ])
}

struct Fixture {
    _temp: Option<tempfile::TempDir>,
    root: PathBuf,
    catalog: PathBuf,
    game: PathBuf,
}

impl Fixture {
    fn new(payload: &[u8]) -> Self {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path().join("manager");
        let mut fixture = Self::at(root, payload);
        fixture._temp = Some(temp);
        fixture
    }

    fn at(root: PathBuf, payload: &[u8]) -> Self {
        let base = root
            .parent()
            .expect("fixture root has a parent")
            .to_path_buf();
        let mut fixture = Self {
            _temp: None,
            root,
            catalog: base.join("catalog"),
            game: base.join("game"),
        };
        fixture.setup(payload);
        fixture
    }

    fn setup(&mut self, payload: &[u8]) {
        fs::create_dir_all(self.root.join("buckets/main/games")).expect("main bucket");
        fs::write(
            self.root.join("buckets/main/bucket.json"),
            r#"{"schema_version":1,"name":"main","title":"Fixture"}"#,
        )
        .expect("main bucket metadata");
        fs::create_dir_all(self.catalog.join("games/test-game")).expect("catalog");
        fs::write(
            self.catalog.join("bucket.json"),
            r#"{"schema_version":1,"name":"fixture","title":"Fixture"}"#,
        )
        .expect("catalog metadata");
        fs::create_dir_all(self.game.join("bin")).expect("game");
        fs::write(self.game.join("bin/TestGame.exe"), b"synthetic marker").expect("game marker");
        self.release("1", FILE_ID, payload, true);
        let catalog = self.catalog.to_str().expect("UTF-8 catalog path");
        self.run(&["bucket", "add", "fixture", catalog])
            .expect("register fixture catalog");
    }

    fn release(&self, version: &str, file_id: u64, payload: &[u8], current: bool) -> String {
        let archive = self
            .root
            .parent()
            .expect("fixture root has a parent")
            .join(format!("nexusmod-{version}.zip"));
        let mut zip = zip::ZipWriter::new(fs::File::create(&archive).expect("archive"));
        zip.start_file(
            "Demo/package/plugins/nexus.plugin",
            zip::write::SimpleFileOptions::default(),
        )
        .expect("archive entry");
        zip.write_all(payload).expect("archive payload");
        zip.finish().expect("archive finish");
        let hash = format!(
            "{:x}",
            Sha256::digest(fs::read(&archive).expect("archive bytes"))
        );
        let manifest = json!({
            "schema_version": 1, "game": "test-game", "id": "nexusmod",
            "name": "Nexus Mod", "version": version, "archive_format": "zip",
            "sha256": hash,
            "source": {
                "kind": "nexus", "game_domain": DOMAIN, "mod_id": MOD_ID,
                "file_id": file_id, "file_name": "NexusMod.zip"
            }
        });
        let name = if current {
            "nexusmod.json".to_owned()
        } else {
            format!("nexusmod@{version}.json")
        };
        fs::write(
            self.catalog.join("games/test-game").join(name),
            serde_json::to_vec(&manifest).expect("manifest JSON"),
        )
        .expect("manifest");
        hash
    }

    fn run(&self, args: &[&str]) -> anyhow::Result<()> {
        let mut cli =
            Cli::try_parse_from(std::iter::once("modn").chain(args.iter().copied())).expect("CLI");
        crate::prepare_nxm(&mut cli.command)?;
        let module = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/test-game");
        let state = self.root.join("state.sqlite");
        data_dir::with_data_directory(&self.root, || {
            with_module_source(Some(&module), || crate::run_cli(cli, &state))
        })
    }

    fn path(&self) -> &str {
        self.game.to_str().expect("UTF-8 game path")
    }

    fn archive_bytes(&self, version: &str) -> Vec<u8> {
        fs::read(
            self.root
                .parent()
                .expect("fixture root has a parent")
                .join(format!("nexusmod-{version}.zip")),
        )
        .expect("archive bytes")
    }

    fn reset_state_and_game(&self) {
        for name in ["state.sqlite", "state.sqlite-wal", "state.sqlite-shm"] {
            let path = self.root.join(name);
            if path.exists() {
                fs::remove_file(path).expect("remove state");
            }
        }
        fs::remove_dir_all(&self.game).expect("remove game");
        fs::create_dir_all(self.game.join("bin")).expect("game");
        fs::write(self.game.join("bin/TestGame.exe"), b"synthetic marker").expect("game marker");
    }
}

fn with_api_key<T>(run: impl FnOnce() -> T) -> T {
    let key = NexusApiKey::new(API_KEY).expect("valid fixture key");
    nexus_auth::with_test_key(TestKey::Present(key), run)
}

fn with_missing_key<T>(run: impl FnOnce() -> T) -> T {
    nexus_auth::with_test_key(TestKey::Missing, run)
}

fn error_of(result: anyhow::Result<()>) -> String {
    format!("{:#}", result.expect_err("command should fail"))
}

fn header<'a>(request: &'a RecordedRequest, name: &str) -> Option<&'a str> {
    request
        .headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn assert_no_secrets(text: &str) {
    for secret in SECRETS {
        assert!(
            !text.contains(secret),
            "secret {secret} leaked into: {text}"
        );
    }
}

fn assert_bytes_have_no_secrets(bytes: &[u8], what: &Path) {
    for secret in SECRETS {
        assert!(
            !contains(bytes, secret.as_bytes()),
            "secret {secret} leaked into {}",
            what.display()
        );
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn scan_state_tree(root: &Path) {
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let entries =
            fs::read_dir(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        for entry in entries {
            let entry = entry.expect("directory entry");
            let name = entry.file_name().to_string_lossy().into_owned();
            for secret in SECRETS {
                assert!(
                    !name.contains(secret),
                    "secret {secret} leaked into file name {}",
                    entry.path().display()
                );
            }
            let metadata = fs::symlink_metadata(entry.path()).expect("metadata");
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                let bytes = fs::read(entry.path()).expect("file bytes");
                assert_bytes_have_no_secrets(&bytes, &entry.path());
            }
        }
    }
}

#[test]
fn free_path_downloads_with_the_nxm_grant_and_installs() {
    let fixture = Fixture::new(b"v1");
    let expires = expires();
    nexus_download::set_responses(free_responses(
        FILE_ID,
        expires,
        CDN_URL,
        &fixture.archive_bytes("1"),
    ));
    with_api_key(|| {
        fixture.run(&[
            "install",
            "nexusmod",
            "--path",
            fixture.path(),
            "--yes",
            "--nxm",
            &nxm_link(FILE_ID, expires),
        ])
    })
    .expect("free-path install");
    assert_eq!(
        fs::read(fixture.game.join("addons/nexus.plugin")).expect("installed file"),
        b"v1"
    );
    let requests = nexus_download::requests();
    let api = requests
        .iter()
        .find(|request| request.url.starts_with("https://api.nexusmods.com/"))
        .expect("API request");
    assert!(api.url.contains(&format!("key={NXM_KEY}")), "{}", api.url);
    assert!(
        api.url.contains(&format!("expires={expires}")),
        "{}",
        api.url
    );
    assert_eq!(header(api, "apikey"), Some(API_KEY));
    assert_eq!(header(api, "application-name"), Some("Modn"));
    assert_eq!(
        header(api, "application-version"),
        Some(env!("CARGO_PKG_VERSION"))
    );
    let cdn = requests
        .iter()
        .find(|request| request.url == CDN_URL)
        .expect("CDN request");
    assert_eq!(header(cdn, "apikey"), None);
}

#[test]
fn premium_path_downloads_without_a_grant_and_installs() {
    let fixture = Fixture::new(b"v1");
    nexus_download::set_responses(premium_responses(
        FILE_ID,
        CDN_URL,
        &fixture.archive_bytes("1"),
    ));
    with_api_key(|| fixture.run(&["install", "nexusmod", "--path", fixture.path(), "--yes"]))
        .expect("premium install");
    assert_eq!(
        fs::read(fixture.game.join("addons/nexus.plugin")).expect("installed file"),
        b"v1"
    );
    let requests = nexus_download::requests();
    let api = requests
        .iter()
        .find(|request| request.url.starts_with("https://api.nexusmods.com/"))
        .expect("API request");
    assert!(!api.url.contains('?'), "{}", api.url);
    assert_eq!(header(api, "apikey"), Some(API_KEY));
    let cdn = requests
        .iter()
        .find(|request| request.url == CDN_URL)
        .expect("CDN request");
    assert_eq!(header(cdn, "apikey"), None);
}

#[test]
fn nexus_status_errors_are_explained_without_secrets() {
    for (status, expected) in [
        (401, "rejected the API key"),
        (403, "free account"),
        (410, "download link expired"),
        (429, "rate limit"),
    ] {
        let fixture = Fixture::new(b"v1");
        let grant = (status != 403).then(|| expires());
        nexus_download::set_responses(response_map([(
            api_url(FILE_ID, grant),
            FixtureResponse::status(status),
        )]));
        let link = grant.map(|expires| nxm_link(FILE_ID, expires));
        let mut args = vec!["install", "nexusmod", "--path", fixture.path(), "--yes"];
        if let Some(link) = &link {
            args.extend(["--nxm", link.as_str()]);
        }
        let message = with_api_key(|| error_of(fixture.run(&args)));
        assert!(message.contains(expected), "{status}: {message}");
        assert_no_secrets(&message);
        assert!(!fixture.game.join("addons").exists());
        assert!(nexus_download::requests()
            .iter()
            .all(|request| { request.url.starts_with("https://api.nexusmods.com/") }));
    }
}

#[test]
fn second_install_reuses_the_verified_cache_without_requests_or_key() {
    let fixture = Fixture::new(b"v1");
    let expires = expires();
    nexus_download::set_responses(free_responses(
        FILE_ID,
        expires,
        CDN_URL,
        &fixture.archive_bytes("1"),
    ));
    with_api_key(|| {
        fixture.run(&[
            "install",
            "nexusmod",
            "--path",
            fixture.path(),
            "--yes",
            "--nxm",
            &nxm_link(FILE_ID, expires),
        ])
    })
    .expect("first install");
    fixture.reset_state_and_game();
    nexus_download::set_responses(BTreeMap::new());
    with_missing_key(|| fixture.run(&["install", "nexusmod", "--path", fixture.path(), "--yes"]))
        .expect("cached install");
    assert!(nexus_download::requests().is_empty());
    assert_eq!(
        fs::read(fixture.game.join("addons/nexus.plugin")).expect("installed file"),
        b"v1"
    );
}

#[test]
fn a_pin_mismatch_from_the_cdn_is_refused() {
    let fixture = Fixture::new(b"v1");
    let expires = expires();
    nexus_download::set_responses(free_responses(FILE_ID, expires, CDN_URL, b"tampered"));
    let message = with_api_key(|| {
        error_of(fixture.run(&[
            "install",
            "nexusmod",
            "--path",
            fixture.path(),
            "--yes",
            "--nxm",
            &nxm_link(FILE_ID, expires),
        ]))
    });
    assert!(message.contains("hash check failed"), "{message}");
    assert_no_secrets(&message);
    assert!(!fixture.game.join("addons").exists());
}

#[test]
fn an_unmatched_nxm_link_is_refused() {
    let fixture = Fixture::new(b"v1");
    nexus_download::set_responses(BTreeMap::new());
    let message = with_missing_key(|| {
        error_of(fixture.run(&[
            "install",
            "nexusmod",
            "--path",
            fixture.path(),
            "--yes",
            "--nxm",
            &nxm_link(FILE_ID + 1, expires()),
        ]))
    });
    assert!(
        message.contains(&format!("nxm link for mod {MOD_ID} file {}", FILE_ID + 1)),
        "{message}"
    );
    assert!(nexus_download::requests().is_empty());
    assert!(!fixture.game.join("addons").exists());
}

#[test]
fn a_missing_api_key_is_refused() {
    let fixture = Fixture::new(b"v1");
    nexus_download::set_responses(BTreeMap::new());
    let message = with_missing_key(|| {
        error_of(fixture.run(&["install", "nexusmod", "--path", fixture.path(), "--yes"]))
    });
    assert_eq!(message, nexus_auth::MISSING_KEY);
    assert!(nexus_download::requests().is_empty());
    assert!(!fixture.game.join("addons").exists());
}

#[test]
fn an_api_redirect_is_refused() {
    let fixture = Fixture::new(b"v1");
    nexus_download::set_responses(response_map([(
        api_url(FILE_ID, None),
        FixtureResponse::redirect("https://api.nexusmods.com/v1/other"),
    )]));
    let message = with_api_key(|| {
        error_of(fixture.run(&["install", "nexusmod", "--path", fixture.path(), "--yes"]))
    });
    assert!(message.contains("must not redirect"), "{message}");
    assert_no_secrets(&message);
    assert!(!fixture.game.join("addons").exists());
}

#[test]
fn a_cdn_redirect_to_a_non_nexus_host_is_refused() {
    let fixture = Fixture::new(b"v1");
    nexus_download::set_responses(response_map([
        (
            api_url(FILE_ID, None),
            FixtureResponse::ok(api_link_body(CDN_URL)),
        ),
        (
            CDN_URL.to_owned(),
            FixtureResponse::redirect("https://evil.invalid/NexusMod.zip"),
        ),
    ]));
    let message = with_api_key(|| {
        error_of(fixture.run(&["install", "nexusmod", "--path", fixture.path(), "--yes"]))
    });
    assert!(message.contains("not a Nexus host"), "{message}");
    assert_no_secrets(&message);
    assert!(!fixture.game.join("addons").exists());
}

#[test]
fn update_downloads_the_new_release_and_reuses_its_cache_without_a_key() {
    let fixture = Fixture::new(b"v1");
    let first = expires();
    nexus_download::set_responses(free_responses(
        FILE_ID,
        first,
        CDN_URL,
        &fixture.archive_bytes("1"),
    ));
    with_api_key(|| {
        fixture.run(&[
            "install",
            "nexusmod",
            "--path",
            fixture.path(),
            "--yes",
            "--nxm",
            &nxm_link(FILE_ID, first),
        ])
    })
    .expect("install v1");
    fixture.release("1", FILE_ID, b"v1", false);
    let second_file = FILE_ID + 1;
    let second_cdn = "https://nexus-cdn.com/files/NexusMod-2.zip?token=SECRETCDN789";
    fixture.release("2", second_file, b"v2", true);
    let second = expires();
    nexus_download::set_responses(free_responses(
        second_file,
        second,
        second_cdn,
        &fixture.archive_bytes("2"),
    ));
    with_api_key(|| {
        fixture.run(&[
            "update",
            "nexusmod",
            "--path",
            fixture.path(),
            "--yes",
            "--nxm",
            &nxm_link(second_file, second),
        ])
    })
    .expect("update to v2");
    assert_eq!(
        fs::read(fixture.game.join("addons/nexus.plugin")).expect("updated file"),
        b"v2"
    );
    assert!(nexus_download::requests().iter().any(|request| {
        request
            .url
            .contains(&format!("files/{second_file}/download_link.json"))
    }));

    fixture.release("2", second_file, b"v2", false);
    let third_file = second_file + 1;
    let third_cdn = "https://nexus-cdn.com/files/NexusMod-3.zip?token=SECRETCDN789";
    fixture.release("3", third_file, b"v3", true);
    let third = expires();
    nexus_download::set_responses(free_responses(
        third_file,
        third,
        third_cdn,
        &fixture.archive_bytes("3"),
    ));
    with_api_key(|| {
        fixture.run(&[
            "update",
            "nexusmod",
            "--path",
            fixture.path(),
            "--dry-run",
            "--download",
            "--nxm",
            &nxm_link(third_file, third),
        ])
    })
    .expect("download v3 into the cache");
    assert_eq!(
        fs::read(fixture.game.join("addons/nexus.plugin")).expect("installed file"),
        b"v2"
    );

    nexus_download::set_responses(BTreeMap::new());
    with_missing_key(|| fixture.run(&["update", "nexusmod", "--path", fixture.path(), "--yes"]))
        .expect("cached update");
    assert!(nexus_download::requests().is_empty());
    assert_eq!(
        fs::read(fixture.game.join("addons/nexus.plugin")).expect("updated file"),
        b"v3"
    );
}

#[test]
fn a_dry_run_without_download_needs_no_key_or_request() {
    let fixture = Fixture::new(b"v1");
    nexus_download::set_responses(BTreeMap::new());
    with_missing_key(|| {
        fixture.run(&["install", "nexusmod", "--path", fixture.path(), "--dry-run"])
    })
    .expect("dry run");
    assert!(nexus_download::requests().is_empty());
    assert!(!fixture.game.join("addons").exists());
}

#[test]
fn free_path_keeps_secrets_out_of_output_and_state() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path().join("manager");
    let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .arg("--exact")
        .arg("nexus_fixture_tests::nexus_fixture_child")
        .arg("--ignored")
        .arg("--nocapture")
        .env("MODN_NEXUS_FIXTURE_ROOT", &root)
        .output()
        .expect("run fixture child");
    assert!(
        output.status.success(),
        "child failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut text = output.stdout;
    text.extend_from_slice(&output.stderr);
    assert_bytes_have_no_secrets(&text, Path::new("<child output>"));
    let output_text = String::from_utf8_lossy(&text);
    assert!(
        output_text.contains("Nexus Mods: cyberpunk2077 mod 6945 file 123456 (NexusMod.zip)"),
        "{output_text}"
    );
    let cache = root.join("cache");
    assert!(
        fs::read_dir(&cache)
            .expect("cache directory")
            .filter_map(std::result::Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().contains('#')),
        "the free-path install wrote a cache entry"
    );
    assert!(root.join("state.sqlite").is_file());
    scan_state_tree(&root);
}

#[test]
#[ignore = "spawned by free_path_keeps_secrets_out_of_output_and_state"]
fn nexus_fixture_child() {
    let Some(root) = std::env::var_os("MODN_NEXUS_FIXTURE_ROOT").map(PathBuf::from) else {
        return;
    };
    let fixture = Fixture::at(root, b"v1");
    let expires = expires();
    nexus_download::set_responses(free_responses(
        FILE_ID,
        expires,
        CDN_URL,
        &fixture.archive_bytes("1"),
    ));
    with_api_key(|| {
        fixture.run(&[
            "install",
            "nexusmod",
            "--path",
            fixture.path(),
            "--yes",
            "--nxm",
            &nxm_link(FILE_ID, expires),
        ])
    })
    .expect("fixture install");
}
