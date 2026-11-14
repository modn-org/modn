#![allow(
    dead_code,
    reason = "shared fixture exposes helpers for different test crates"
)]
use minisign::KeyPair;
use std::ffi::OsStr;
use std::fs;
use std::io::Cursor;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use tempfile::TempDir;

/// The `modn` binary that tests run.
///
/// Windows locks a running executable, so a concurrent `cargo build` cannot
/// replace `target/debug/modn.exe` while tests run it. On Windows, each test
/// crate runs its own copy instead.
pub fn modn_binary() -> PathBuf {
    let source = Path::new(env!("CARGO_BIN_EXE_modn"));
    if !cfg!(windows) {
        return source.to_owned();
    }
    static COPY: OnceLock<PathBuf> = OnceLock::new();
    COPY.get_or_init(|| crate_copy(source)).clone()
}

fn crate_copy(source: &Path) -> PathBuf {
    let test_crate = std::env::current_exe().expect("test executable");
    let folder = source
        .parent()
        .expect("binary folder")
        .join("test-binaries")
        .join(test_crate.file_stem().expect("test executable name"));
    let copy = folder.join(source.file_name().expect("binary name"));
    if !same_build(source, &copy) {
        fs::create_dir_all(&folder).expect("binary copy folder");
        fs::copy(source, &copy).expect("copy of the modn binary");
    }
    copy
}

fn same_build(source: &Path, copy: &Path) -> bool {
    let (Ok(source), Ok(copy)) = (fs::metadata(source), fs::metadata(copy)) else {
        return false;
    };
    source.len() == copy.len() && source.modified().ok() == copy.modified().ok()
}

fn program(binary: &OsStr) -> PathBuf {
    if Path::new(binary) == Path::new(env!("CARGO_BIN_EXE_modn")) {
        modn_binary()
    } else {
        PathBuf::from(binary)
    }
}

fn keypair() -> &'static KeyPair {
    static KEY: OnceLock<KeyPair> = OnceLock::new();
    KEY.get_or_init(|| KeyPair::generate_unencrypted_keypair().expect("fixture signing key"))
}

pub fn test_public_key() -> String {
    keypair()
        .pk
        .to_box()
        .expect("fixture public key")
        .into_string()
}

fn sign(pair: &KeyPair, root: &Path) -> String {
    let statement = modn_core::buckets::catalog_statement(root).expect("fixture catalog statement");
    minisign::sign(
        Some(&pair.pk),
        &pair.sk,
        Cursor::new(statement.as_bytes()),
        Some("main bucket fixture"),
        Some("main bucket fixture signature"),
    )
    .expect("fixture signature")
    .into_string()
}

pub fn write_signature(root: &Path, zip: &Path) {
    fs::write(format!("{}.minisig", zip.display()), sign(keypair(), root))
        .expect("fixture signature file");
}

pub fn write_foreign_signature(root: &Path, zip: &Path) {
    let pair = KeyPair::generate_unencrypted_keypair().expect("foreign fixture signing key");
    fs::write(format!("{}.minisig", zip.display()), sign(&pair, root))
        .expect("fixture signature file");
}

/// Rewrite a copied v1 main bucket as a contract v2 fixture.
///
/// The checked-in `buckets/main` submodule stays v1. Test fixtures move
/// package records under `mods/<game>/` and map the v1 fields to v2 fields.
pub fn to_contract_v2(root: &Path) {
    let metadata_path = root.join("bucket.json");
    let mut metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(&metadata_path).expect("bucket metadata"))
            .expect("bucket JSON");
    metadata["schema_version"] = serde_json::json!(2);
    fs::write(&metadata_path, serde_json::to_vec(&metadata).expect("JSON"))
        .expect("bucket metadata");

    let games = root.join("games");
    let entries = match fs::read_dir(&games) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => panic!("cannot read fixture games: {error}"),
    };
    for game in entries {
        let game = game.expect("fixture game");
        if !game.file_type().expect("fixture type").is_dir() {
            continue;
        }
        let mods = root.join("mods").join(game.file_name());
        for record in fs::read_dir(game.path()).expect("fixture records") {
            let record = record.expect("fixture record");
            let path = record.path();
            if path
                .file_name()
                .is_some_and(|name| name == "module.json" || name == "module.lua")
                || path.extension().is_none_or(|extension| extension != "json")
            {
                continue;
            }
            let mut value: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).expect("fixture record"))
                    .expect("record JSON");
            value["schema_version"] = serde_json::json!(2);
            let source = value
                .as_object_mut()
                .and_then(|record| record.remove("source"));
            if let Some(source) = source {
                value["sources"] = serde_json::json!([source]);
            }
            let pin = value
                .as_object_mut()
                .and_then(|record| record.remove("sha256"));
            if let Some(pin) = pin {
                value["blake3"] = pin;
            }
            fs::create_dir_all(&mods).expect("mods directory");
            fs::write(
                mods.join(record.file_name()),
                serde_json::to_vec(&value).expect("JSON"),
            )
            .expect("v2 record");
            fs::remove_file(&path).expect("remove v1 record");
        }
    }
}

pub fn seed(data: &Path) {
    fn copy(source: &Path, target: &Path) {
        fs::create_dir_all(target).expect("bucket directory");
        for entry in fs::read_dir(source).expect("bucket entries") {
            let entry = entry.expect("bucket entry");
            let target = target.join(entry.file_name());
            if entry.file_type().expect("entry type").is_dir() {
                copy(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).expect("bucket file");
            }
        }
    }
    let main = data.join("modn/buckets/main");
    copy(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../buckets/main"),
        &main,
    );
    to_contract_v2(&main);
}

pub struct SeededCommand {
    command: Command,
    _data: TempDir,
}

impl Deref for SeededCommand {
    type Target = Command;
    fn deref(&self) -> &Command {
        &self.command
    }
}

impl DerefMut for SeededCommand {
    fn deref_mut(&mut self) -> &mut Command {
        &mut self.command
    }
}

pub fn command(binary: impl AsRef<OsStr>) -> SeededCommand {
    let data = tempfile::tempdir().expect("data directory");
    seed(data.path());
    let mut command = Command::new(program(binary.as_ref()));
    command
        .env("LOCALAPPDATA", data.path())
        .env("MODN_TEST_MAIN_BUCKET_PUBLIC_KEY", test_public_key());
    SeededCommand {
        command,
        _data: data,
    }
}
