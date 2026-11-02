#![allow(
    dead_code,
    reason = "each integration-test crate uses a different subset of this fixture"
)]
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;
use zip::write::SimpleFileOptions;

pub struct Fixture {
    pub temp: TempDir,
    pub state: PathBuf,
    pub modules: PathBuf,
    pub game: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().expect("fixture");
        let state = temp.path().join("manager");
        let modules = temp.path().join("modules");
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/test-game");
        let module = modules.join("test-game");
        fs::create_dir_all(&module).expect("module dir");
        for file in ["main.lua", "module.json"] {
            fs::copy(fixture.join(file), module.join(file)).expect("module source");
        }
        let cyberpunk = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/modn-core/game-modules/cyberpunk-2077");
        let cyberpunk_target = modules.join("cyberpunk-2077");
        fs::create_dir_all(&cyberpunk_target).expect("cyberpunk module");
        for file in ["main.lua", "module.json"] {
            fs::copy(cyberpunk.join(file), cyberpunk_target.join(file)).expect("cyberpunk source");
        }
        let game = temp.path().join("game");
        fs::create_dir_all(game.join("bin")).expect("game");
        fs::write(game.join("bin/TestGame.exe"), b"game-exe").expect("marker");
        Self {
            temp,
            state,
            modules,
            game,
        }
    }

    pub fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_modn"))
            .arg("--state-dir")
            .arg(&self.state)
            .env("MODN_GAME_MODULES_DIR", &self.modules)
            .args(args)
            .output()
            .expect("run CLI")
    }

    pub fn run_paths(&self, args: &[&str], paths: &[&Path]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_modn"));
        command
            .arg("--state-dir")
            .arg(&self.state)
            .env("MODN_GAME_MODULES_DIR", &self.modules)
            .args(args);
        for path in paths {
            command.arg(path);
        }
        command.output().expect("run CLI")
    }

    pub fn register(&self) {
        let result = self.run_paths(&["game", "add", "test-game", "--path"], &[&self.game]);
        assert!(result.status.success(), "{}", error(&result));
    }

    pub fn bucket(&self, name: &str, game: &str, id: &str, hash: &str, dependencies: &[&str]) {
        let root = self.temp.path().join(name);
        let manifest_dir = root.join("games").join(game);
        fs::create_dir_all(&manifest_dir).expect("bucket dirs");
        fs::write(
            root.join("bucket.json"),
            serde_json::to_vec(&json!({
                "schema_version": 1, "name":name, "title":"Fixture"
            }))
            .expect("JSON"),
        )
        .expect("bucket metadata");
        fs::write(
            manifest_dir.join(format!("{id}.json")),
            serde_json::to_vec(&json!({
                "schema_version": 1, "id":id, "game":game, "name":"Fixture",
                "version":"1.0", "archive_format":"zip", "sha256":hash,
                "source":{"kind":"manual","filename":"fixture.zip"},
                "dependencies":dependencies
            }))
            .expect("JSON"),
        )
        .expect("manifest");
        let result = self.run_paths(&["bucket", "add", name], &[&root]);
        assert!(result.status.success(), "{}", error(&result));
    }

    pub fn archive(&self) -> (PathBuf, String) {
        let path = self.temp.path().join("fixture.zip");
        let file = fs::File::create(&path).expect("archive");
        let mut zip = zip::ZipWriter::new(file);
        use std::io::Write;
        zip.start_file(
            "Demo/package/plugins/example.plugin",
            SimpleFileOptions::default(),
        )
        .expect("entry");
        zip.write_all(b"installed-by-modn").expect("payload");
        zip.finish().expect("zip finish");
        let digest = Sha256::digest(fs::read(&path).expect("bytes"));
        (path, format!("{digest:x}"))
    }
}

pub fn error(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}
