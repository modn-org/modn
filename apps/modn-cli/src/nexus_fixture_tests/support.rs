use crate::commands::Cli;
use crate::download::nexus_download::{FixtureResponse, RecordedRequest};
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

pub(super) const API_KEY: &str = "SECRETAPIKEY123";
pub(super) const NXM_KEY: &str = "SECRETNXMKEY456";
const CDN_TOKEN: &str = "SECRETCDN789";
const SECRETS: [&str; 3] = [API_KEY, NXM_KEY, CDN_TOKEN];
const DOMAIN: &str = "cyberpunk2077";
pub(super) const MOD_ID: u64 = 6945;
pub(super) const FILE_ID: u64 = 123456;
pub(super) const CDN_URL: &str = "https://nexus-cdn.com/files/NexusMod.zip?token=SECRETCDN789";

pub(super) fn api_url(file_id: u64, grant: Option<u64>) -> String {
    let base = format!(
        "https://api.nexusmods.com/v1/games/{DOMAIN}/mods/{MOD_ID}/files/{file_id}/download_link.json"
    );
    match grant {
        Some(expires) => format!("{base}?key={NXM_KEY}&expires={expires}"),
        None => base,
    }
}

pub(super) fn expires() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after the Unix epoch")
        .as_secs()
        + 3600
}

pub(super) fn nxm_link(file_id: u64, expires: u64) -> String {
    format!("nxm://{DOMAIN}/mods/{MOD_ID}/files/{file_id}?key={NXM_KEY}&expires={expires}")
}

pub(super) fn response_map(
    entries: impl IntoIterator<Item = (String, FixtureResponse)>,
) -> BTreeMap<String, FixtureResponse> {
    entries.into_iter().collect()
}

pub(super) fn api_link_body(cdn: &str) -> Vec<u8> {
    serde_json::to_vec(&json!([{ "URI": cdn }])).expect("JSON")
}

pub(super) fn free_responses(
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

pub(super) fn premium_responses(
    file_id: u64,
    cdn: &str,
    body: &[u8],
) -> BTreeMap<String, FixtureResponse> {
    response_map([
        (
            api_url(file_id, None),
            FixtureResponse::ok(api_link_body(cdn)),
        ),
        (cdn.to_owned(), FixtureResponse::ok(body.to_vec())),
    ])
}

pub(super) struct Fixture {
    _temp: Option<tempfile::TempDir>,
    pub(super) root: PathBuf,
    pub(super) catalog: PathBuf,
    pub(super) game: PathBuf,
}

impl Fixture {
    pub(super) fn new(payload: &[u8]) -> Self {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path().join("manager");
        let mut fixture = Self::at(root, payload);
        fixture._temp = Some(temp);
        fixture
    }

    pub(super) fn at(root: PathBuf, payload: &[u8]) -> Self {
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

    pub(super) fn release(
        &self,
        version: &str,
        file_id: u64,
        payload: &[u8],
        current: bool,
    ) -> String {
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

    pub(super) fn run(&self, args: &[&str]) -> anyhow::Result<()> {
        let mut cli =
            Cli::try_parse_from(std::iter::once("modn").chain(args.iter().copied())).expect("CLI");
        crate::prepare_nxm(&mut cli.command)?;
        let module = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/test-game");
        let state = self.root.join("state.sqlite");
        data_dir::with_data_directory(&self.root, || {
            with_module_source(Some(&module), || {
                crate::run_cli(cli, &state, &crate::progress::Progress::start(None))
            })
        })
    }

    pub(super) fn path(&self) -> &str {
        self.game.to_str().expect("UTF-8 game path")
    }

    pub(super) fn archive_bytes(&self, version: &str) -> Vec<u8> {
        fs::read(
            self.root
                .parent()
                .expect("fixture root has a parent")
                .join(format!("nexusmod-{version}.zip")),
        )
        .expect("archive bytes")
    }

    pub(super) fn reset_state_and_game(&self) {
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

pub(super) fn with_api_key<T>(run: impl FnOnce() -> T) -> T {
    let key = NexusApiKey::new(API_KEY).expect("valid fixture key");
    nexus_auth::with_test_key(TestKey::Present(key), run)
}

pub(super) fn with_missing_key<T>(run: impl FnOnce() -> T) -> T {
    nexus_auth::with_test_key(TestKey::Missing, run)
}

pub(super) fn error_of(result: anyhow::Result<()>) -> String {
    format!("{:#}", result.expect_err("command should fail"))
}

pub(super) fn header<'a>(request: &'a RecordedRequest, name: &str) -> Option<&'a str> {
    request
        .headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

pub(super) fn assert_no_secrets(text: &str) {
    for secret in SECRETS {
        assert!(
            !text.contains(secret),
            "secret {secret} leaked into: {text}"
        );
    }
}

pub(super) fn assert_bytes_have_no_secrets(bytes: &[u8], what: &Path) {
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

pub(super) fn scan_state_tree(root: &Path) {
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
