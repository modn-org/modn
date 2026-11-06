#![allow(
    dead_code,
    reason = "each integration-test crate uses a different subset of this fixture"
)]
//! Shared fixture for API 2 modules that declare a content root.
//!
//! Every command runs the real `modn` binary against a temporary game folder,
//! a temporary appdata base and a temporary state directory. `MODN_APPDATA`
//! and `APPDATA` both point at the temporary base, so no test reads or writes
//! the user's real `%APPDATA%`.

use crate::main_bucket;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Output;
use tempfile::TempDir;
use zip::write::SimpleFileOptions;

pub const GAME_ID: &str = "content-game";
pub const CONTENT_RELATIVE: &str = "Fixture/Mods";

/// API 2 policy that maps every archive file to a content-root destination.
const SCRIPT: &str = r#"
local fs = mods_host.fs

return {
    game_id = "content-game", api_version = 2, version = "1.0.0",
    marker = "ContentGame.exe", common_paths = {},
    validate = function()
        local ok, info = pcall(fs.metadata, 1, "ContentGame.exe")
        return ok and info.kind == "file"
    end,
    steam_name = function() return "Content Game" end,
    is_epic = function() return false end,
    scan = function()
        return { mods = {}, framework_signals = {}, incomplete = false, redmod_available = false }
    end,
    doctor = function() return {} end,
    plan = function(files)
        local operations = {}
        for _, file in ipairs(files) do
            table.insert(operations, {
                source = file.source,
                destination = "Pkg/" .. file.source,
                kind = "content-mod",
                package_root = "",
                root = "content",
            })
        end
        return { schema_version = 1, operations = operations, blockers = {} }
    end,
}
"#;

pub struct Fixture {
    pub temp: TempDir,
    pub game: PathBuf,
    pub appdata: PathBuf,
    pub state: PathBuf,
    pub modules: PathBuf,
    bucket_root: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().expect("fixture");
        let game = temp.path().join("game");
        let appdata = temp.path().join("appdata");
        let state = temp.path().join("manager");
        let modules = temp.path().join("modules");
        fs::create_dir_all(&game).expect("game");
        fs::write(game.join("ContentGame.exe"), b"marker").expect("marker");
        fs::create_dir_all(&appdata).expect("appdata");
        let module = modules.join(GAME_ID);
        fs::create_dir_all(&module).expect("module");
        fs::write(
            module.join("module.json"),
            format!(
                r#"{{"game_id":"{GAME_ID}","version":"1.0.0","api_version":2,"entrypoint":"module.lua","content_root":{{"base":"appdata","path":"{CONTENT_RELATIVE}"}}}}"#
            ),
        )
        .expect("manifest");
        fs::write(module.join("module.lua"), SCRIPT).expect("script");
        let cyberpunk =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../buckets/main/games/cyberpunk-2077");
        let cyberpunk_target = modules.join("cyberpunk-2077");
        fs::create_dir_all(&cyberpunk_target).expect("official module");
        for file in ["module.json", "module.lua"] {
            fs::copy(cyberpunk.join(file), cyberpunk_target.join(file)).expect("official source");
        }
        Self {
            temp,
            game,
            appdata,
            state,
            modules,
            bucket_root: PathBuf::new(),
        }
    }

    pub fn run(&self, args: &[&str]) -> Output {
        self.run_env(args, &[])
    }

    pub fn run_env(&self, args: &[&str], environment: &[(&str, &str)]) -> Output {
        let mut command = main_bucket::command(env!("CARGO_BIN_EXE_modn"));
        command
            .arg("--state-dir")
            .arg(&self.state)
            .env("MODN_GAME_MODULES_DIR", &self.modules)
            .env("MODN_APPDATA", &self.appdata)
            .env("APPDATA", &self.appdata);
        for (name, value) in environment {
            command.env(name, value);
        }
        command.args(args).output().expect("run modn")
    }

    pub fn ok(&self, args: &[&str]) -> Output {
        let output = self.run(args);
        assert!(output.status.success(), "{}", error(&output));
        output
    }

    pub fn json(&self, args: &[&str]) -> Value {
        serde_json::from_slice(&self.ok(args).stdout).expect("JSON output")
    }

    pub fn register(&self) {
        self.ok(&[
            "game",
            "add",
            GAME_ID,
            "--path",
            self.game.to_str().expect("game path"),
        ]);
    }

    pub fn content_root(&self) -> PathBuf {
        self.appdata.join(CONTENT_RELATIVE)
    }

    pub fn content(&self, relative: &str) -> PathBuf {
        self.content_root().join(relative)
    }

    pub fn database(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.state.join("state.sqlite")).expect("state database")
    }

    pub fn archive(&self, name: &str, files: &[(&str, &[u8])]) -> (PathBuf, String) {
        let path = self.temp.path().join(format!("{name}.zip"));
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).expect("archive"));
        for (file, bytes) in files {
            zip.start_file(*file, SimpleFileOptions::default())
                .expect("entry");
            zip.write_all(bytes).expect("payload");
        }
        zip.finish().expect("finish archive");
        let digest = Sha256::digest(fs::read(&path).expect("archive bytes"));
        (path, format!("{digest:x}"))
    }

    pub fn bucket(&mut self) {
        let root = self.temp.path().join("fixture-bucket");
        fs::create_dir_all(root.join(format!("games/{GAME_ID}"))).expect("bucket dirs");
        fs::write(
            root.join("bucket.json"),
            serde_json::to_vec(&json!({
                "schema_version": 1, "name": "fixture-bucket", "title": "Content root fixture"
            }))
            .expect("JSON"),
        )
        .expect("bucket metadata");
        self.ok(&[
            "bucket",
            "add",
            "fixture-bucket",
            root.to_str().expect("bucket path"),
        ]);
        self.bucket_root = root;
    }

    pub fn release(&self, file: &str, version: &str, hash: &str) {
        fs::write(
            self.bucket_root.join("games").join(GAME_ID).join(file),
            serde_json::to_vec(&json!({
                "schema_version": 1, "id": "fixture", "game": GAME_ID,
                "name": "Fixture", "version": version, "archive_format": "zip",
                "sha256": hash,
                "source": {"kind": "manual", "filename": format!("fixture-{version}.zip")}
            }))
            .expect("JSON"),
        )
        .expect("release manifest");
    }
}

pub fn error(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().expect("canonical path")
}

/// Resolve the declared root the same way the planner does, even before it exists.
pub fn expected_content_root(fixture: &Fixture) -> PathBuf {
    canonical(&fixture.appdata).join(CONTENT_RELATIVE)
}

pub fn ownership_key(path: &str) -> String {
    if cfg!(windows) {
        path.chars().flat_map(char::to_lowercase).collect()
    } else {
        path.to_owned()
    }
}

pub fn owned_files(database: &rusqlite::Connection) -> i64 {
    database
        .query_row("SELECT COUNT(*) FROM managed_files", [], |row| row.get(0))
        .expect("owned file count")
}

pub fn assert_content_ownership(database: &rusqlite::Connection, root: &Path) {
    let mut roots = database
        .prepare("SELECT root, path FROM installation_roots ORDER BY root")
        .expect("query roots");
    let rows: Vec<(String, String)> = roots
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("roots")
        .collect::<rusqlite::Result<_>>()
        .expect("root rows");
    assert!(
        rows.iter()
            .any(|(kind, path)| kind == "content" && Path::new(path) == canonical(root)),
        "content root not recorded: {rows:?}"
    );
    let mut files = database
        .prepare("SELECT root, relative_path, path_key FROM managed_files ORDER BY path_key")
        .expect("query files");
    let owners: Vec<(String, String, String)> = files
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .expect("files")
        .collect::<rusqlite::Result<_>>()
        .expect("file rows");
    assert!(!owners.is_empty(), "no owned content files recorded");
    for (kind, relative, key) in owners {
        assert_eq!(kind, "content", "ownership must use the content root");
        assert_eq!(
            key,
            ownership_key(&relative),
            "path keys stay root-relative and case-folded"
        );
    }
}
