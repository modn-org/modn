#![allow(
    dead_code,
    reason = "each theme test module uses a different subset of this fixture"
)]
//! Temporary data, state and theme fixtures for the theme CLI tests.

use crate::main_bucket;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

pub const GAME: &str = "cyberpunk-2077";

/// One test run with its own appdata base, state directory and theme sources.
pub struct Fixture {
    pub temp: TempDir,
    pub data: PathBuf,
    pub state: PathBuf,
    pub sources: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().expect("fixture");
        let data = temp.path().join("appdata");
        let state = temp.path().join("manager");
        let sources = temp.path().join("sources");
        fs::create_dir_all(&data).expect("appdata");
        fs::create_dir_all(&sources).expect("sources");
        Self {
            temp,
            data,
            state,
            sources,
        }
    }

    /// The shared data directory that holds the official `main` bucket copy.
    pub fn data_dir(&self) -> PathBuf {
        self.data.join("modn")
    }

    pub fn main_themes(&self) -> PathBuf {
        self.data_dir().join("buckets").join("main").join("themes")
    }

    /// A command with the documented desktop prefix and temporary state.
    pub fn command(&self) -> Command {
        let mut command = Command::new(main_bucket::modn_binary());
        command
            .arg("--json")
            .arg("--state-dir")
            .arg(&self.state)
            .env("LOCALAPPDATA", &self.data)
            .env("APPDATA", &self.data)
            .env("MODN_APPDATA", &self.data);
        command
    }

    pub fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().expect("run modn")
    }

    /// Run without `--json` for the human-readable paths.
    pub fn run_human(&self, args: &[&str]) -> Output {
        let mut command = Command::new(main_bucket::modn_binary());
        command
            .arg("--state-dir")
            .arg(&self.state)
            .env("LOCALAPPDATA", &self.data)
            .env("APPDATA", &self.data)
            .env("MODN_APPDATA", &self.data)
            .args(args)
            .output()
            .expect("run modn")
    }

    pub fn json(&self, args: &[&str]) -> Value {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("JSON output")
    }

    pub fn failed(&self, args: &[&str]) -> Output {
        let output = self.run(args);
        assert!(
            !output.status.success(),
            "command unexpectedly succeeded: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        output
    }

    pub fn error_json(&self, args: &[&str]) -> Value {
        let output = self.failed(args);
        serde_json::from_slice(&output.stderr).expect("JSON error")
    }

    pub fn register(&self, source: &Path) -> Value {
        self.json(&["theme", "register", path(source)])
    }

    /// A valid theme in a source tree shaped as `<game>/<id>`.
    pub fn source_theme(&self, game: &str, id: &str, variants: &[&str]) -> PathBuf {
        let directory = self.sources.join(game).join(id);
        write_theme(&directory, game, id, "Night City", variants, VALID_CSS);
        directory
    }

    /// A valid theme below the official `main` bucket copy.
    pub fn official_theme(&self, game: &str, id: &str, variants: &[&str]) -> PathBuf {
        let directory = self.main_themes().join(game).join(id);
        write_theme(&directory, game, id, "Official City", variants, VALID_CSS);
        directory
    }

    /// A broken theme below the official `main` bucket copy.
    pub fn broken_official_theme(&self, game: &str, id: &str) -> PathBuf {
        let directory = self.main_themes().join(game).join(id);
        write_theme(
            &directory,
            game,
            id,
            "Broken City",
            &[],
            ".modn-app { position: fixed; }",
        );
        directory
    }

    /// A valid theme in one registered bucket, with its registration file.
    pub fn bucket_theme(&self, bucket: &str, game: &str, id: &str, variants: &[&str]) -> PathBuf {
        let root = self.temp.path().join("bucket").join(bucket);
        fs::create_dir_all(root.join("mods")).expect("bucket mods");
        fs::write(
            root.join("bucket.json"),
            serde_json::to_vec(&json!({
                "schema_version": 2,
                "name": bucket,
                "title": "Theme fixture bucket"
            }))
            .expect("bucket JSON"),
        )
        .expect("bucket metadata");
        let directory = root.join("themes").join(game).join(id);
        write_theme(&directory, game, id, "Bucket City", variants, VALID_CSS);
        self.add_bucket(bucket, &root);
        directory
    }

    /// Write the registration that `modn bucket add` stores for a local bucket.
    pub fn add_bucket(&self, bucket: &str, root: &Path) {
        let registrations = self.state.join("buckets").join("registrations");
        fs::create_dir_all(&registrations).expect("registrations");
        fs::write(
            registrations.join(format!("{bucket}.json")),
            serde_json::to_vec(&json!({"path": root})).expect("registration JSON"),
        )
        .expect("registration");
    }

    pub fn registry_entry(&self, game: &str, id: &str) -> PathBuf {
        self.state
            .join("themes")
            .join("registry")
            .join(game)
            .join(id)
    }
}

/// The simple stylesheet that the CSS checker accepts.
pub const VALID_CSS: &str = ".modn-app { color: #ffffff; }";

fn path(value: &Path) -> &str {
    value.to_str().expect("test path is UTF-8")
}

fn write_theme(
    directory: &Path,
    game: &str,
    id: &str,
    name: &str,
    variants: &[&str],
    css: &str,
) {
    fs::create_dir_all(directory).expect("theme directory");
    fs::write(
        directory.join("theme.json"),
        serde_json::to_vec(&json!({
            "schema_version": 1,
            "id": id,
            "game": game,
            "name": name,
            "theme_api": 1,
            "variants": variants,
            "icon": null
        }))
        .expect("theme JSON"),
    )
    .expect("theme manifest");
    fs::write(directory.join("theme.css"), css).expect("theme css");
    for variant in variants {
        fs::write(
            directory.join(format!("theme.{variant}.css")),
            ".modn-card { border-color: #00e5ff; }",
        )
        .expect("variant css");
    }
}
