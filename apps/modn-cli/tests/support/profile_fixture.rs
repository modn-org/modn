//! Disposable game installation, manager profile and local archives for the
//! `modn profile` CLI tests.
//!
//! [`ProfileFixture::new`] registers `cyberpunk-2077` with the official `main`
//! bucket seeded inside a temporary `--state-dir`. Every child process points
//! `LOCALAPPDATA`, `APPDATA` and `MODN_APPDATA` at the same temporary folder and
//! passes the standard `--game` and `--state-dir` flags. [`install`] adds one
//! real managed package to the active profile, so later commands see the same
//! memberships as a user installation.
//!
//! [`install`]: ProfileFixture::install

#[path = "main_bucket.rs"]
mod main_bucket;

use serde_json::Value;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;
use zip::write::SimpleFileOptions;

/// One temporary game installation with a registered `default` profile.
pub struct ProfileFixture {
    _root: TempDir,
    game: PathBuf,
    state: PathBuf,
    data: PathBuf,
    archives: PathBuf,
}

impl ProfileFixture {
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
        let output = fixture.run(&[
            "game",
            "add",
            "cyberpunk-2077",
            "--path",
            fixture.game.to_str().expect("game path"),
        ]);
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

    /// The temporary manager profile that holds `state.sqlite`.
    pub fn state(&self) -> &Path {
        &self.state
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

    /// Installs one local archive as a managed package and returns its report.
    pub fn install(&self, name: &str) -> Value {
        let archive = self.archives.join(format!("{name}.zip"));
        let file = File::create(&archive).expect("archive");
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(
            format!("archive/pc/mod/{name}.archive"),
            SimpleFileOptions::default(),
        )
        .expect("archive entry");
        zip.write_all(name.as_bytes()).expect("archive payload");
        zip.finish().expect("archive finish");
        self.json(&[
            "install",
            archive.to_str().expect("archive path"),
            "--path",
            self.game.to_str().expect("game path"),
            "--yes",
        ])
    }

    /// Reads `profile list --json` for the registered installation.
    pub fn profile_list(&self) -> Value {
        self.json(&["profile", "list"])
    }
}
