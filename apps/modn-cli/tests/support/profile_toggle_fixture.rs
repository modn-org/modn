//! Disposable game installation for `profile enable` and `profile disable`.
//!
//! Every install writes one shared destination, so the later package wins and
//! disabling it must restore the earlier package's verified archive bytes. Each
//! archive is copied into the manager cache under a valid entry name.

#[path = "main_bucket.rs"]
mod main_bucket;

use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;
use zip::write::SimpleFileOptions;

pub const SHARED: &str = "archive/pc/mod/shared.archive";

/// One temporary game installation with a registered `default` profile.
pub struct ToggleFixture {
    _root: TempDir,
    game: PathBuf,
    state: PathBuf,
    data: PathBuf,
    archives: PathBuf,
}

impl ToggleFixture {
    /// Creates the fixture and registers `cyberpunk-2077` with `modn game add`.
    pub fn new() -> Self {
        let root = tempfile::tempdir().expect("fixture");
        let game = root.path().join("game");
        fs::create_dir_all(game.join("bin/x64")).expect("game directory");
        fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"game").expect("game marker");
        let state = root.path().join("state");
        main_bucket::seed_state(&state);
        let data = root.path().join("data");
        fs::create_dir_all(&data).expect("data directory");
        let archives = root.path().join("archives");
        fs::create_dir_all(&archives).expect("archive directory");
        let game = game.canonicalize().expect("canonical game");
        let fixture = Self {
            _root: root,
            game,
            state,
            data,
            archives,
        };
        let output = fixture
            .command()
            .args(["game", "add", "cyberpunk-2077", "--path"])
            .arg(&fixture.game)
            .output()
            .expect("run the modn binary");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        fixture
    }

    /// The canonical installation directory.
    pub fn game(&self) -> &Path {
        &self.game
    }

    /// The default archive cache of the temporary manager profile.
    pub fn cache(&self) -> PathBuf {
        self.state.join("cache")
    }

    /// A command with the standard game, state and data locations set.
    pub fn command(&self) -> Command {
        let mut command = Command::new(main_bucket::modn_binary());
        command
            .env("LOCALAPPDATA", &self.data)
            .env("APPDATA", &self.data)
            .env("MODN_APPDATA", &self.data)
            .env(
                "MODN_TEST_MAIN_BUCKET_PUBLIC_KEY",
                main_bucket::test_public_key(),
            )
            .arg("--game")
            .arg("cyberpunk-2077")
            .arg("--state-dir")
            .arg(&self.state);
        command
    }

    /// Runs one command without `--json` and returns its captured output.
    pub fn run(&self, args: &[&str]) -> Output {
        self.command()
            .args(args)
            .output()
            .expect("run the modn binary")
    }

    /// Runs one command with `--json` and parses its successful stdout.
    pub fn json(&self, args: &[&str]) -> Value {
        let output = self.command().arg("--json").args(args).output();
        let output = output.expect("run the modn binary");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("JSON output")
    }

    /// Runs one command with `--json` and returns its error code and message.
    pub fn error(&self, args: &[&str]) -> (String, String) {
        let output = self
            .command()
            .arg("--json")
            .args(args)
            .output()
            .expect("run the modn binary");
        assert!(
            !output.status.success(),
            "the command unexpectedly succeeded"
        );
        let value: Value = serde_json::from_slice(&output.stderr).expect("JSON error");
        (
            value["error"].as_str().expect("error code").to_owned(),
            value["message"].as_str().expect("error message").to_owned(),
        )
    }

    /// Installs one local archive that writes [`SHARED`] and returns the report.
    pub fn install(&self, name: &str) -> Value {
        let archive = self.archives.join(format!("{name}.zip"));
        let file = File::create(&archive).expect("archive");
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(SHARED, SimpleFileOptions::default())
            .expect("archive entry");
        zip.write_all(name.as_bytes()).expect("archive payload");
        zip.finish().expect("archive finish");
        let report = self.json(&[
            "install",
            archive.to_str().expect("archive path"),
            "--path",
            self.game.to_str().expect("game path"),
            "--yes",
        ]);
        let cache = self.cache();
        fs::create_dir_all(&cache).expect("cache directory");
        fs::copy(
            &archive,
            cache.join(format!("cyberpunk-2077#{name}#1.0#0000000.zip")),
        )
        .expect("cache entry");
        report
    }

    /// The bytes of the shared game file.
    pub fn shared_bytes(&self) -> Vec<u8> {
        fs::read(self.game.join(SHARED)).expect("shared file")
    }

    /// Every file and directory below the game installation.
    pub fn tree(&self) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
        let mut entries = BTreeMap::new();
        let mut pending = vec![self.game.clone()];
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(&directory).expect("game directory") {
                let path = entry.expect("entry").path();
                let relative = path
                    .strip_prefix(&self.game)
                    .expect("relative path")
                    .to_path_buf();
                if fs::symlink_metadata(&path).expect("metadata").is_dir() {
                    pending.push(path);
                    entries.insert(relative, None);
                } else {
                    entries.insert(relative, Some(fs::read(&path).expect("file bytes")));
                }
            }
        }
        entries
    }

    /// Reads `profile list --json` for the registered installation.
    pub fn profile_list(&self) -> Value {
        self.json(&["profile", "list"])
    }

    /// A writable connection to the temporary state database.
    pub fn database(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.state.join("state.sqlite")).expect("state database")
    }
}
