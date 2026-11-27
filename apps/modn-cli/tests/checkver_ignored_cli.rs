#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "support/catalog.rs"]
mod support;

use serde_json::{json, Value};
use std::fs;
use std::io;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output};
use support::{error, Fixture};

struct Proxy {
    listener: TcpListener,
    address: String,
}

impl Proxy {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("proxy listener");
        listener
            .set_nonblocking(true)
            .expect("nonblocking proxy listener");
        let address = format!("http://{}", listener.local_addr().expect("proxy address"));
        Self { listener, address }
    }

    fn accepted(&self) -> usize {
        let mut count = 0;
        loop {
            match self.listener.accept() {
                Ok(_) => count += 1,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return count,
                Err(error) => panic!("proxy accept failed: {error}"),
            }
        }
    }
}

fn base_command() -> Command {
    Command::new(main_bucket::modn_binary())
}

fn with_proxy(mut command: Command, proxy: &Proxy, fixture: &Fixture) -> Command {
    command
        .env("LOCALAPPDATA", fixture.temp.path())
        .env("APPDATA", fixture.temp.path())
        .env("MODN_APPDATA", fixture.temp.path())
        .env("HTTP_PROXY", &proxy.address)
        .env("HTTPS_PROXY", &proxy.address)
        .env("ALL_PROXY", &proxy.address);
    command
}

fn run_state(fixture: &Fixture, proxy: &Proxy, args: &[&str], paths: &[&Path]) -> Output {
    let mut command = with_proxy(base_command(), proxy, fixture);
    command
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .args(args);
    for path in paths {
        command.arg(path);
    }
    command.output().expect("run CLI")
}

fn run_plain(fixture: &Fixture, proxy: &Proxy, args: &[&str], paths: &[&Path]) -> Output {
    let mut command = with_proxy(base_command(), proxy, fixture);
    command.args(args);
    for path in paths {
        command.arg(path);
    }
    command.output().expect("run CLI")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("command JSON output")
}

fn path_text(path: &Path) -> &str {
    path.to_str().expect("test fixture invariant")
}

fn manifest(id: &str, pin: &str, checkver: Option<Value>) -> Value {
    let mut manifest = json!({
        "schema_version": 2,
        "id": id,
        "game": "test-game",
        "name": format!("Fixture {id}"),
        "version": "1.0",
        "archive_format": "zip",
        "blake3": pin,
        "sources": [{"kind": "manual", "filename": "fixture.zip"}],
        "dependencies": []
    });
    if let Some(checkver) = checkver {
        manifest["checkver"] = checkver;
    }
    manifest
}

fn checkver_blocks() -> [(&'static str, Value); 3] {
    [
        (
            "alpha",
            json!({
                "kind": "github-release",
                "repository": "owner/alpha",
                "asset_pattern": "Alpha-*.zip",
                "prerelease": false
            }),
        ),
        (
            "beta",
            json!({
                "kind": "git",
                "repository": "owner/beta",
                "branch": "main"
            }),
        ),
        (
            "gamma",
            json!({
                "kind": "nexus",
                "game_domain": "cyberpunk2077",
                "mod_id": 6945,
                "file_pattern": "Gamma-?.7z"
            }),
        ),
    ]
}

fn write_bucket(root: &Path, pin: &str, with_checkver: bool) {
    let records = root.join("mods/test-game");
    fs::create_dir_all(&records).expect("bucket records");
    fs::write(
        root.join("bucket.json"),
        r#"{"schema_version":2,"name":"community","title":"Community"}"#,
    )
    .expect("bucket metadata");
    for (id, block) in checkver_blocks() {
        let checkver = with_checkver.then_some(block);
        fs::write(
            records.join(format!("{id}.json")),
            serde_json::to_vec(&manifest(id, pin, checkver)).expect("manifest JSON"),
        )
        .expect("manifest");
    }
}

fn search(fixture: &Fixture, proxy: &Proxy, json_output: bool) -> Output {
    let mut args = Vec::new();
    if json_output {
        args.push("--json");
    }
    args.extend(["--game", "test-game", "search"]);
    run_state(fixture, proxy, &args, &[])
}

fn dry_run(fixture: &Fixture, proxy: &Proxy, archive: &Path) -> Output {
    run_state(
        fixture,
        proxy,
        &[
            "--json",
            "install",
            "community/test-game/alpha",
            "--dry-run",
            "--file",
        ],
        &[archive],
    )
}

fn without_checkver(value: &mut Value) {
    let entries = value.as_array_mut().expect("search result array");
    for entry in entries {
        entry
            .as_object_mut()
            .expect("search result object")
            .remove("checkver");
    }
}

#[test]
fn invalid_checkver_names_the_package_and_field() {
    let fixture = Fixture::new();
    let proxy = Proxy::new();
    let (_, pin) = fixture.archive();
    let bucket = fixture.temp.path().join("community");
    write_bucket(&bucket, &pin, false);
    let register = run_state(
        &fixture,
        &proxy,
        &["game", "add", "test-game", "--path"],
        &[&fixture.game],
    );
    assert!(register.status.success(), "{}", error(&register));
    let add = run_state(
        &fixture,
        &proxy,
        &["bucket", "add", "community"],
        &[&bucket],
    );
    assert!(add.status.success(), "{}", error(&add));

    let invalid = manifest(
        "alpha",
        &pin,
        Some(json!({
            "kind": "github-release",
            "repository": "owner",
            "asset_pattern": "Alpha-*.zip"
        })),
    );
    fs::write(
        bucket.join("mods/test-game/alpha.json"),
        serde_json::to_vec(&invalid).expect("manifest JSON"),
    )
    .expect("invalid manifest");

    let search = search(&fixture, &proxy, false);
    assert!(!search.status.success());
    let message = error(&search);
    assert!(
        message.contains("package test-game/alpha"),
        "package not named: {message}"
    );
    assert!(
        message.contains("checkver.repository"),
        "field not named: {message}"
    );
    assert_eq!(proxy.accepted(), 0);
}

#[test]
fn checkver_blocks_are_ignored_by_user_commands_and_authoring_commands() {
    let fixture = Fixture::new();
    let proxy = Proxy::new();
    let (archive, pin) = fixture.archive();
    let bucket = fixture.temp.path().join("community");
    let keys = fixture.temp.path().join("keys");
    let key = keys.join("author.key");
    let public_key = bucket.join("catalog.pub");

    let keygen = run_plain(
        &fixture,
        &proxy,
        &[
            "--json",
            "catalog",
            "keygen",
            "--out",
            path_text(&keys),
            "--name",
            "author",
        ],
        &[],
    );
    assert!(keygen.status.success(), "{}", error(&keygen));

    let register = run_state(
        &fixture,
        &proxy,
        &["game", "add", "test-game", "--path"],
        &[&fixture.game],
    );
    assert!(register.status.success(), "{}", error(&register));

    write_bucket(&bucket, &pin, true);
    let add = run_state(
        &fixture,
        &proxy,
        &["bucket", "add", "community"],
        &[&bucket],
    );
    assert!(add.status.success(), "{}", error(&add));

    let with_text = search(&fixture, &proxy, false);
    assert!(with_text.status.success(), "{}", error(&with_text));
    let with_json = search(&fixture, &proxy, true);
    assert!(with_json.status.success(), "{}", error(&with_json));
    let with_dry_run = dry_run(&fixture, &proxy, &archive);
    assert!(with_dry_run.status.success(), "{}", error(&with_dry_run));
    let with_hash = run_plain(&fixture, &proxy, &["catalog", "hash"], &[&archive]);
    assert!(with_hash.status.success(), "{}", error(&with_hash));
    let with_sign = run_plain(
        &fixture,
        &proxy,
        &["--json", "catalog", "sign", path_text(&bucket), "--key"],
        &[&key],
    );
    assert!(with_sign.status.success(), "{}", error(&with_sign));
    let with_verify = run_plain(
        &fixture,
        &proxy,
        &["--json", "catalog", "verify", path_text(&bucket), "--key"],
        &[&public_key],
    );
    assert!(with_verify.status.success(), "{}", error(&with_verify));

    let mut with_search = json(&with_json);
    for entry in with_search.as_array().expect("search results") {
        assert!(
            entry.get("checkver").is_some(),
            "search output keeps manifest metadata: {entry}"
        );
    }
    without_checkver(&mut with_search);

    write_bucket(&bucket, &pin, false);
    let without_text = search(&fixture, &proxy, false);
    assert!(without_text.status.success(), "{}", error(&without_text));
    let without_json = search(&fixture, &proxy, true);
    assert!(without_json.status.success(), "{}", error(&without_json));
    let without_dry_run = dry_run(&fixture, &proxy, &archive);
    assert!(
        without_dry_run.status.success(),
        "{}",
        error(&without_dry_run)
    );
    let without_hash = run_plain(&fixture, &proxy, &["catalog", "hash"], &[&archive]);
    assert!(without_hash.status.success(), "{}", error(&without_hash));
    let without_sign = run_plain(
        &fixture,
        &proxy,
        &["--json", "catalog", "sign", path_text(&bucket), "--key"],
        &[&key],
    );
    assert!(without_sign.status.success(), "{}", error(&without_sign));
    let without_verify = run_plain(
        &fixture,
        &proxy,
        &["--json", "catalog", "verify", path_text(&bucket), "--key"],
        &[&public_key],
    );
    assert!(
        without_verify.status.success(),
        "{}",
        error(&without_verify)
    );

    let mut without_search = json(&without_json);
    without_checkver(&mut without_search);
    assert_eq!(stdout(&with_text), stdout(&without_text));
    assert_eq!(with_search, without_search);
    assert_eq!(json(&with_dry_run), json(&without_dry_run));
    assert_eq!(stdout(&with_hash), stdout(&without_hash));
    assert_eq!(
        json(&with_sign)["fingerprint"],
        json(&without_sign)["fingerprint"]
    );
    assert_eq!(
        json(&with_verify)["fingerprint"],
        json(&without_verify)["fingerprint"]
    );
    assert_eq!(
        proxy.accepted(),
        0,
        "user commands opened a network connection"
    );
}
