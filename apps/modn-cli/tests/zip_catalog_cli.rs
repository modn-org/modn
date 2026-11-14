#![allow(
    clippy::too_many_lines,
    reason = "end-to-end acceptance cases share one fixture"
)]
#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "support/zip_catalog.rs"]
mod zip_catalog;
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;
use zip_catalog::{fixture_zip, listing, pack, snapshot};
const SIGNED_KEY: &str = "2A7E5A370638B878";
const REKEYED_KEY: &str = "DC69368386F82F95";
type StateSnapshot = (Vec<u8>, Vec<(String, Vec<u8>)>, Vec<String>);
struct Fixture {
    temp: TempDir,
    data: PathBuf,
    state: PathBuf,
    main_zip: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new().expect("test fixture invariant");
        let data = temp.path().join("data");
        fs::create_dir_all(&data).expect("data directory");
        main_bucket::seed(&data);
        let tree = temp.path().join("main-tree");
        zip_catalog::copy_catalog(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../buckets/main"),
            &tree,
        );
        main_bucket::to_contract_v2(&tree);
        let main_zip = temp.path().join("main.zip");
        pack(&tree, &main_zip);
        main_bucket::write_signature(&tree, &main_zip);
        let state = temp.path().join("state");
        Self {
            temp,
            data,
            state,
            main_zip,
        }
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(main_bucket::modn_binary())
            .env("LOCALAPPDATA", &self.data)
            .env(
                "MODN_MAIN_BUCKET_SOURCE",
                self.main_zip.to_str().expect("main ZIP path"),
            )
            .env(
                "MODN_TEST_MAIN_BUCKET_PUBLIC_KEY",
                main_bucket::test_public_key(),
            )
            .env_remove("MODN_GAME_MODULES_DIR")
            .args(["--state-dir", self.state.to_str().expect("state path")])
            .args(args)
            .output()
            .expect("run fixture command")
    }
    fn success(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("fixture command UTF-8 output")
    }
    fn sync(&self, zip: &Path) -> Output {
        self.run(&[
            "--json",
            "bucket",
            "sync",
            "community",
            "--source",
            zip.to_str().expect("ZIP path"),
        ])
    }
    fn registration(&self) -> PathBuf {
        self.state.join("buckets/registrations/community.json")
    }
    fn catalog(&self) -> PathBuf {
        let bytes = fs::read(self.registration()).expect("registration file");
        let value: Value = serde_json::from_slice(&bytes).expect("registration JSON");
        PathBuf::from(value["path"].as_str().expect("registration path"))
    }
    fn state_snapshot(&self) -> StateSnapshot {
        (
            fs::read(self.registration()).expect("registration file"),
            snapshot(&self.catalog()),
            listing(&self.state.join("buckets")),
        )
    }
    fn assert_no_staging(&self) {
        let buckets = self.state.join("buckets");
        assert!(
            !listing(&buckets)
                .iter()
                .any(|name| name.starts_with("zip-catalog-")),
            "staging left behind: {:?}",
            listing(&buckets)
        );
    }
}
fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}
fn listed(fixture: &Fixture) -> Vec<Value> {
    let output = fixture.success(&["--json", "bucket", "list"]);
    serde_json::from_str(&output).expect("bucket list JSON")
}
fn zip_with_entries(path: &Path, entries: &[(&str, &[u8])]) {
    let mut writer = zip::ZipWriter::new(fs::File::create(path).expect("hostile ZIP"));
    for (name, bytes) in entries {
        writer
            .start_file(*name, zip::write::SimpleFileOptions::default())
            .expect("hostile entry");
        writer.write_all(bytes).expect("hostile bytes");
    }
    writer.finish().expect("hostile ZIP complete");
}
/// Write a stored ZIP with raw entry names, including exact duplicates.
fn raw_zip(path: &Path, entries: &[(&str, &[u8])]) {
    fn put(bytes: &mut Vec<u8>, value: &[u8]) {
        bytes.extend_from_slice(value);
    }
    let mut body = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let offset = body.len() as u32;
        let size = data.len() as u32;
        let mut header = Vec::new();
        put(&mut header, &0x0403_4b50u32.to_le_bytes());
        put(&mut header, &20u16.to_le_bytes());
        put(&mut header, &0u16.to_le_bytes());
        put(&mut header, &0u16.to_le_bytes());
        put(&mut header, &0u16.to_le_bytes());
        put(&mut header, &0u16.to_le_bytes());
        put(&mut header, &0u32.to_le_bytes());
        put(&mut header, &size.to_le_bytes());
        put(&mut header, &size.to_le_bytes());
        put(&mut header, &(name.len() as u16).to_le_bytes());
        put(&mut header, &0u16.to_le_bytes());
        put(&mut header, name.as_bytes());
        body.extend_from_slice(&header);
        body.extend_from_slice(data);
        put(&mut central, &0x0201_4b50u32.to_le_bytes());
        put(&mut central, &20u16.to_le_bytes());
        put(&mut central, &20u16.to_le_bytes());
        put(&mut central, &0u16.to_le_bytes());
        put(&mut central, &0u16.to_le_bytes());
        put(&mut central, &0u16.to_le_bytes());
        put(&mut central, &0u16.to_le_bytes());
        put(&mut central, &0u32.to_le_bytes());
        put(&mut central, &size.to_le_bytes());
        put(&mut central, &size.to_le_bytes());
        put(&mut central, &(name.len() as u16).to_le_bytes());
        put(&mut central, &0u16.to_le_bytes());
        put(&mut central, &0u16.to_le_bytes());
        put(&mut central, &0u16.to_le_bytes());
        put(&mut central, &0u16.to_le_bytes());
        put(&mut central, &0u32.to_le_bytes());
        put(&mut central, &offset.to_le_bytes());
        put(&mut central, name.as_bytes());
    }
    let central_offset = body.len() as u32;
    body.extend_from_slice(&central);
    put(&mut body, &0x0605_4b50u32.to_le_bytes());
    put(&mut body, &0u16.to_le_bytes());
    put(&mut body, &0u16.to_le_bytes());
    put(&mut body, &(entries.len() as u16).to_le_bytes());
    put(&mut body, &(entries.len() as u16).to_le_bytes());
    put(&mut body, &(central.len() as u32).to_le_bytes());
    put(&mut body, &central_offset.to_le_bytes());
    put(&mut body, &0u16.to_le_bytes());
    fs::write(path, body).expect("raw ZIP");
}
fn hostile(fixture: &Fixture, name: &str, build: impl FnOnce(&Path)) -> Output {
    let zip = fixture.temp.path().join(format!("{name}.zip"));
    build(&zip);
    fixture.sync(&zip)
}
fn assert_refused_without_state(fixture: &Fixture, output: &Output, expected: &str) {
    assert!(!output.status.success(), "command unexpectedly succeeded");
    assert!(
        stderr(output).contains(expected),
        "expected {expected:?} in {}",
        stderr(output)
    );
    assert!(!fixture.registration().exists(), "registration was written");
    fixture.assert_no_staging();
}
#[test]
fn valid_zip_registers_pins_and_replaces_atomically() {
    let fixture = Fixture::new();
    let first = fixture.sync(&fixture_zip(fixture.temp.path(), "v2"));
    assert!(first.status.success(), "{}", stderr(&first));
    assert!(
        stderr(&first).contains(&format!("Pinned signer key {SIGNED_KEY}")),
        "{}",
        stderr(&first)
    );
    let synced: Value = serde_json::from_slice(&first.stdout).expect("sync JSON");
    assert_eq!(synced["signer"]["key_id"], SIGNED_KEY);
    assert_eq!(synced["pinned"], true);
    let entry = listed(&fixture)
        .into_iter()
        .find(|entry| entry["name"] == "community")
        .expect("registered bucket");
    assert_eq!(entry["source_kind"], "zip");
    assert_eq!(entry["signer_key_id"], SIGNED_KEY);
    let registration: Value =
        serde_json::from_slice(&fs::read(fixture.registration()).expect("registration"))
            .expect("registration JSON");
    assert_eq!(registration["zip"]["signer"]["key_id"], SIGNED_KEY);
    assert!(registration["zip"]["source"]["url"]
        .as_str()
        .expect("ZIP source")
        .ends_with(".zip"));
    let before = fixture.state_snapshot();
    let updated = fixture.temp.path().join("v1-update");
    zip_catalog::copy_catalog(&zip_catalog::catalog_fixture("v1"), &updated);
    let source = fixture_zip(fixture.temp.path(), "v2");
    pack(&updated, &source);
    let second = fixture.sync(&source);
    assert!(second.status.success(), "{}", stderr(&second));
    assert!(!stderr(&second).contains("Pinned signer key"));
    let after = fixture.state_snapshot();
    assert_ne!(before.0, after.0, "registration must change");
    let catalog = fixture.catalog();
    let record = fs::read_to_string(catalog.join("mods/test-game/example.json")).expect("record");
    assert!(record.contains("\"version\":\"1.0\""), "{record}");
}
#[test]
fn unsigned_zip_is_refused_and_writes_nothing() {
    let fixture = Fixture::new();
    let directory = fixture.temp.path().join("unsigned");
    zip_catalog::copy_catalog(&zip_catalog::catalog_fixture("v2"), &directory);
    fs::remove_file(directory.join("catalog.sig")).expect("remove signature");
    let zip = fixture.temp.path().join("unsigned.zip");
    pack(&directory, &zip);
    let output = fixture.sync(&zip);
    assert_refused_without_state(
        &fixture,
        &output,
        "is unsigned; Modn refuses unsigned catalogs",
    );
}
#[test]
fn tampered_zip_keeps_previous_registration_and_catalog() {
    let fixture = Fixture::new();
    assert!(fixture
        .sync(&fixture_zip(fixture.temp.path(), "v2"))
        .status
        .success());
    let before = fixture.state_snapshot();
    let directory = fixture.temp.path().join("tampered");
    zip_catalog::copy_catalog(&zip_catalog::catalog_fixture("v1"), &directory);
    let record = directory.join("mods/test-game/example.json");
    let mut bytes = fs::read(&record).expect("record");
    let last = bytes.len() - 2;
    bytes[last] ^= 1;
    fs::write(&record, bytes).expect("tampered record");
    let source = fixture.temp.path().join("v2.zip");
    pack(&directory, &source);
    let output = fixture.sync(&source);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("catalog signature verification failed"),
        "{}",
        stderr(&output)
    );
    assert_eq!(before, fixture.state_snapshot());
}
#[test]
fn wrong_or_missing_public_key_is_refused() {
    let fixture = Fixture::new();
    let directory = fixture.temp.path().join("wrong-key");
    zip_catalog::copy_catalog(&zip_catalog::catalog_fixture("v2"), &directory);
    fs::copy(
        zip_catalog::catalog_fixture("v2-rekeyed").join("catalog.pub"),
        directory.join("catalog.pub"),
    )
    .expect("foreign public key");
    let zip = fixture.temp.path().join("wrong-key.zip");
    pack(&directory, &zip);
    let output = fixture.sync(&zip);
    assert_refused_without_state(&fixture, &output, "does not match public key");
    let directory = fixture.temp.path().join("missing-key");
    zip_catalog::copy_catalog(&zip_catalog::catalog_fixture("v2"), &directory);
    fs::remove_file(directory.join("catalog.pub")).expect("remove public key");
    let zip = fixture.temp.path().join("missing-key.zip");
    pack(&directory, &zip);
    let output = fixture.sync(&zip);
    assert_refused_without_state(
        &fixture,
        &output,
        "is unsigned; Modn refuses unsigned catalogs",
    );
}
#[test]
fn different_signer_on_resync_is_refused() {
    let fixture = Fixture::new();
    assert!(fixture
        .sync(&fixture_zip(fixture.temp.path(), "v2"))
        .status
        .success());
    let before = fixture.state_snapshot();
    let directory = fixture.temp.path().join("rekeyed-update");
    zip_catalog::copy_catalog(&zip_catalog::catalog_fixture("v2-rekeyed"), &directory);
    let source = fixture.temp.path().join("v2.zip");
    pack(&directory, &source);
    let output = fixture.sync(&source);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains(&format!(
            "Signer key changed from {SIGNED_KEY} to {REKEYED_KEY}"
        )),
        "{}",
        stderr(&output)
    );
    assert_eq!(before, fixture.state_snapshot());
}
#[test]
fn revision_and_plain_http_are_refused_without_network() {
    let fixture = Fixture::new();
    let zip = fixture_zip(fixture.temp.path(), "v2");
    let output = fixture.run(&[
        "bucket",
        "sync",
        "community",
        "--source",
        zip.to_str().expect("ZIP path"),
        "--revision",
        "main",
    ]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("`--revision` cannot be used with a ZIP catalog source"),
        "{}",
        stderr(&output)
    );
    let output = fixture.run(&[
        "bucket",
        "sync",
        "community",
        "--source",
        "http://example.org/catalog.zip",
    ]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("must use https://; plain HTTP is refused"),
        "{}",
        stderr(&output)
    );
    assert!(!fixture.registration().exists());
}
#[test]
fn update_refreshes_zip_catalog_and_keeps_previous_on_failure() {
    let fixture = Fixture::new();
    let source = fixture_zip(fixture.temp.path(), "v2");
    assert!(fixture.sync(&source).status.success());
    let updated = fixture_zip(fixture.temp.path(), "v1");
    fs::copy(&updated, &source).expect("replace hosted ZIP");
    let report: Value =
        serde_json::from_str(&fixture.success(&["--json", "update"])).expect("update JSON");
    assert_eq!(report["failed"], 0);
    let community = report["buckets"]
        .as_array()
        .expect("bucket array")
        .iter()
        .find(|entry| entry["name"] == "community")
        .expect("community refresh");
    assert_eq!(community["kind"], "zip");
    assert_eq!(community["changed"], true);
    let digest = community["digest"].as_str().expect("digest").to_owned();
    let before = fixture.state_snapshot();
    let directory = fixture.temp.path().join("tampered");
    zip_catalog::copy_catalog(&zip_catalog::catalog_fixture("v2"), &directory);
    let record = directory.join("mods/test-game/example.json");
    let mut bytes = fs::read(&record).expect("record");
    let last = bytes.len() - 2;
    bytes[last] ^= 1;
    fs::write(&record, bytes).expect("tampered record");
    let tampered = fixture.temp.path().join("tampered.zip");
    pack(&directory, &tampered);
    fs::copy(&tampered, &source).expect("host tampered ZIP");
    let output = fixture.run(&["update"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("1 of 2 catalog refreshes failed"),
        "{}",
        stderr(&output)
    );
    assert_eq!(before, fixture.state_snapshot());
    let registration: Value =
        serde_json::from_slice(&fs::read(fixture.registration()).expect("registration"))
            .expect("registration JSON");
    assert_eq!(registration["zip"]["digest"], digest);
}
#[test]
fn contract_v2_violations_are_refused() {
    let fixture = Fixture::new();
    let keys = fixture.temp.path().join("keys");
    let keygen = Command::new(main_bucket::modn_binary())
        .args(["catalog", "keygen", "--out"])
        .arg(&keys)
        .output()
        .expect("keygen");
    assert!(keygen.status.success(), "{}", stderr(&keygen));
    let directory = fixture.temp.path().join("v1-contract");
    zip_catalog::copy_catalog(&zip_catalog::catalog_fixture("v2"), &directory);
    fs::write(
        directory.join("bucket.json"),
        r#"{"schema_version":1,"name":"community","title":"Old"}"#,
    )
    .expect("v1 metadata");
    let sign = Command::new(main_bucket::modn_binary())
        .args(["catalog", "sign"])
        .arg(&directory)
        .arg("--key")
        .arg(keys.join("catalog.key"))
        .output()
        .expect("sign v1");
    assert!(sign.status.success(), "{}", stderr(&sign));
    let zip = fixture.temp.path().join("v1-contract.zip");
    pack(&directory, &zip);
    let output = fixture.sync(&zip);
    assert_refused_without_state(&fixture, &output, "requires bucket contract v2");
    let directory = fixture.temp.path().join("empty-sources");
    zip_catalog::copy_catalog(&zip_catalog::catalog_fixture("v2"), &directory);
    fs::write(
        directory.join("mods/test-game/example.json"),
        r#"{"schema_version":2,"id":"example","game":"test-game","name":"Example","version":"1.0","archive_format":"zip","blake3":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","sources":[]}"#,
    )
    .expect("empty sources record");
    let sign = Command::new(main_bucket::modn_binary())
        .args(["catalog", "sign"])
        .arg(&directory)
        .arg("--key")
        .arg(keys.join("catalog.key"))
        .output()
        .expect("sign empty sources");
    assert!(sign.status.success(), "{}", stderr(&sign));
    let zip = fixture.temp.path().join("empty-sources.zip");
    pack(&directory, &zip);
    let output = fixture.sync(&zip);
    assert!(!output.status.success());
    assert!(!fixture.registration().exists());
}
#[test]
fn hostile_zip_entries_are_refused() {
    let fixture = Fixture::new();
    for (name, entry, expected) in [
        ("traversal", "../escape", "unsafe archive entry"),
        ("absolute", "/escape", "unsafe archive entry"),
        ("backslash", "mods\\escape", "backslash separator"),
    ] {
        let output = hostile(&fixture, name, |zip| {
            zip_with_entries(zip, &[(entry, b"x")])
        });
        assert_refused_without_state(&fixture, &output, expected);
        assert!(!fixture.temp.path().join("escape").exists());
    }
    let output = hostile(&fixture, "duplicate", |zip| {
        raw_zip(zip, &[("bucket.json", b"{}"), ("bucket.json", b"{}")])
    });
    assert_refused_without_state(&fixture, &output, "duplicate");
    let output = hostile(&fixture, "case", |zip| {
        zip_with_entries(zip, &[("Mods/a", b"x"), ("mods/a", b"y")])
    });
    assert_refused_without_state(&fixture, &output, "case-colliding");
}
#[test]
fn hostile_zip_symlink_and_limits_are_refused() {
    let fixture = Fixture::new();
    let zip = fixture.temp.path().join("symlink.zip");
    let mut writer = zip::ZipWriter::new(fs::File::create(&zip).expect("symlink ZIP"));
    writer
        .start_file(
            "link",
            zip::write::SimpleFileOptions::default().unix_permissions(0o120777),
        )
        .expect("symlink entry");
    writer.write_all(b"target").expect("symlink bytes");
    writer.finish().expect("symlink ZIP");
    let output = fixture.sync(&zip);
    assert_refused_without_state(&fixture, &output, "symlink");
    let output = hostile(&fixture, "ratio", |zip| {
        let mut writer = zip::ZipWriter::new(fs::File::create(zip).expect("ratio ZIP"));
        writer
            .start_file("zeros", zip::write::SimpleFileOptions::default())
            .expect("ratio entry");
        writer.write_all(&vec![0u8; 1024 * 1024]).expect("zeros");
        writer.finish().expect("ratio ZIP");
    });
    assert_refused_without_state(&fixture, &output, "expansion ratio");
    let output = hostile(&fixture, "oversized", |zip| {
        let mut writer = zip::ZipWriter::new(fs::File::create(zip).expect("oversized ZIP"));
        writer
            .start_file("big", zip::write::SimpleFileOptions::default())
            .expect("oversized entry");
        for _ in 0..65 {
            writer
                .write_all(&vec![0u8; 1024 * 1024])
                .expect("big bytes");
        }
        writer.finish().expect("oversized ZIP");
    });
    assert_refused_without_state(&fixture, &output, "64 MiB size limit");
}
#[test]
fn hostile_zip_entry_count_is_refused() {
    let fixture = Fixture::new();
    let output = hostile(&fixture, "many", |zip| {
        let mut writer = zip::ZipWriter::new(fs::File::create(zip).expect("many ZIP"));
        for index in 0..20_001 {
            writer
                .start_file(
                    format!("entries/{index}.txt"),
                    zip::write::SimpleFileOptions::default(),
                )
                .expect("many entry");
        }
        writer.finish().expect("many ZIP");
    });
    assert_refused_without_state(&fixture, &output, "more than 20000 entries");
}
