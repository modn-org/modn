//! `modn game detect` with fixture Steam, GOG and Epic libraries.
//!
//! `MODN_STORE_LIBRARIES` names a JSON file of fixture library locations, so
//! no test reads a real store library. The real main bucket supplies the
//! Cyberpunk 2077 and Balatro store identities.

#[path = "support/main_bucket.rs"]
mod main_bucket;

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;
use tempfile::TempDir;

struct Libraries {
    temp: TempDir,
    steam: PathBuf,
    gog: PathBuf,
    epic: PathBuf,
    refused: PathBuf,
}

fn write(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().expect("parent")).expect("fixture folder");
    fs::write(path, bytes).expect("fixture file");
}

fn vdf(path: &Path) -> String {
    path.display().to_string().replace('\\', "\\\\")
}

fn app_manifest(library: &Path, app_id: &str, folder: &str) {
    write(
        &library.join(format!("steamapps/appmanifest_{app_id}.acf")),
        format!(
            "\"AppState\"\n{{\n\t\"appid\"\t\t\"{app_id}\"\n\t\"installdir\"\t\t\"{folder}\"\n}}\n"
        )
        .as_bytes(),
    );
}

fn cyberpunk(folder: &Path) -> PathBuf {
    write(&folder.join("bin/x64/Cyberpunk2077.exe"), b"fixture");
    folder.canonicalize().expect("canonical game")
}

impl Libraries {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("library fixtures");
        let root = temp.path().join("Steam");
        let extra = temp.path().join("SteamLibrary");
        write(
            &root.join("steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
                vdf(&root),
                vdf(&extra)
            )
            .as_bytes(),
        );
        app_manifest(&extra, "1091500", "Cyberpunk 2077");
        let steam = cyberpunk(&extra.join("steamapps/common/Cyberpunk 2077"));
        app_manifest(&root, "2379780", "Balatro");
        let refused = root.join("steamapps/common/Balatro");
        fs::create_dir_all(&refused).expect("refused folder");

        let gog = cyberpunk(&temp.path().join("GOG Games/Cyberpunk 2077"));
        let epic = cyberpunk(&temp.path().join("Epic Games/Cyberpunk2077"));
        let manifests = temp.path().join("Manifests");
        write(
            &manifests.join("ginger.item"),
            json!({"AppName": "Ginger", "MainGameAppName": "Ginger", "InstallLocation": epic})
                .to_string()
                .as_bytes(),
        );
        write(
            &temp.path().join("libraries.json"),
            json!({
                "steam_roots": [root],
                "epic_manifest_dirs": [manifests],
                "gog_records": [{"product_id": "1423049311", "path": gog}],
            })
            .to_string()
            .as_bytes(),
        );
        Self {
            temp,
            steam,
            gog,
            epic,
            refused,
        }
    }

    fn file(&self) -> PathBuf {
        self.temp.path().join("libraries.json")
    }

    fn games(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut files = BTreeMap::new();
        for root in [&self.steam, &self.gog, &self.epic, &self.refused] {
            collect(root, &mut files);
        }
        files
    }
}

fn collect(root: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
    for entry in fs::read_dir(root).expect("game folder") {
        let entry = entry.expect("game entry");
        if entry.file_type().expect("entry type").is_dir() {
            collect(&entry.path(), files);
        } else {
            files.insert(entry.path(), fs::read(entry.path()).expect("game file"));
        }
    }
}

struct Cli {
    data: TempDir,
    libraries: Libraries,
}

impl Cli {
    fn new() -> Self {
        let data = tempfile::tempdir().expect("data directory");
        main_bucket::seed(data.path());
        Self {
            data,
            libraries: Libraries::new(),
        }
    }

    fn state(&self) -> PathBuf {
        self.data.path().join("modn/state.sqlite")
    }

    fn run(&self, args: &[&str]) -> Output {
        std::process::Command::new(main_bucket::modn_binary())
            .env("LOCALAPPDATA", self.data.path())
            .env("APPDATA", self.data.path())
            .env(
                "MODN_TEST_MAIN_BUCKET_PUBLIC_KEY",
                main_bucket::test_public_key(),
            )
            .env("MODN_STORE_LIBRARIES", self.libraries.file())
            .env_remove("MODN_GAME_MODULES_DIR")
            .args(args)
            .output()
            .expect("run modn")
    }

    fn json(&self, args: &[&str]) -> Value {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("JSON output")
    }
}

fn path_of(value: &Value) -> PathBuf {
    PathBuf::from(value.as_str().expect("path string"))
}

fn installations(report: &Value) -> Vec<(String, String, String, PathBuf, String)> {
    report["installations"]
        .as_array()
        .expect("installations")
        .iter()
        .map(|entry| {
            (
                entry["game_id"].as_str().expect("game").to_owned(),
                entry["store"].as_str().expect("store").to_owned(),
                entry["store_id"].as_str().expect("store id").to_owned(),
                path_of(&entry["path"]),
                entry["status"].as_str().expect("status").to_owned(),
            )
        })
        .collect()
}

fn expected(cli: &Cli, statuses: [&str; 3]) -> Vec<(String, String, String, PathBuf, String)> {
    let libraries = &cli.libraries;
    let mut rows = vec![
        ("steam", "1091500", libraries.steam.clone()),
        ("gog", "1423049311", libraries.gog.clone()),
        ("epic", "Ginger", libraries.epic.clone()),
    ];
    rows.sort_by(|left, right| left.2.cmp(&right.2));
    rows.into_iter()
        .zip(statuses)
        .map(|((store, id, path), status)| {
            (
                "cyberpunk-2077".to_owned(),
                store.to_owned(),
                id.to_owned(),
                path,
                status.to_owned(),
            )
        })
        .collect()
}

#[test]
fn detect_lists_confirmed_installations_of_each_store_without_state() {
    let cli = Cli::new();
    let before = cli.libraries.games();

    let report = cli.json(&["game", "detect", "--json"]);

    assert_eq!(installations(&report), expected(&cli, ["new"; 3]));
    let refused = report["refused"].as_array().expect("refused");
    assert_eq!(refused.len(), 1, "{report:#}");
    assert_eq!(refused[0]["game_id"], "balatro");
    assert_eq!(refused[0]["store"], "steam");
    assert_eq!(refused[0]["store_id"], "2379780");
    assert_eq!(path_of(&refused[0]["path"]), cli.libraries.refused);
    assert!(refused[0]["reason"]
        .as_str()
        .expect("reason")
        .contains("Balatro.exe"));
    assert_eq!(report["skipped"], json!([]));
    assert!(!cli.state().exists(), "detection must not create state");
    assert_eq!(cli.libraries.games(), before);
}

#[test]
fn register_adds_new_installations_once_and_writes_only_manager_state() {
    let cli = Cli::new();
    let before = cli.libraries.games();

    let first = cli.json(&["game", "detect", "--register", "--json"]);
    let second = cli.json(&["game", "detect", "--register", "--json"]);

    assert_eq!(installations(&first), expected(&cli, ["registered"; 3]));
    assert_eq!(
        installations(&second),
        expected(&cli, ["already_registered"; 3])
    );
    let listed = cli.json(&["game", "list", "--json"]);
    let mut sources: Vec<_> = listed
        .as_array()
        .expect("registrations")
        .iter()
        .map(|entry| entry["source"].as_str().expect("source").to_owned())
        .collect();
    sources.sort();
    assert_eq!(sources, ["epic", "gog", "steam"]);
    assert_eq!(cli.libraries.games(), before);
}

#[test]
fn an_installation_registered_by_path_is_reported_and_not_registered_again() {
    let cli = Cli::new();
    let steam = cli.libraries.steam.to_str().expect("steam path").to_owned();
    cli.json(&["game", "add", "cyberpunk-2077", "--path", &steam, "--json"]);

    let report = cli.json(&[
        "--game",
        "cyberpunk-2077",
        "game",
        "detect",
        "--register",
        "--json",
    ]);

    let statuses: Vec<_> = installations(&report)
        .into_iter()
        .map(|(_, store, _, _, status)| (store, status))
        .collect();
    assert!(statuses.contains(&("steam".into(), "already_registered".into())));
    assert_eq!(
        statuses
            .iter()
            .filter(|(_, status)| status == "registered")
            .count(),
        2
    );
    assert_eq!(report["refused"], json!([]), "--game limits detection");
    let listed = cli.json(&["game", "list", "--json"]);
    let registrations = listed.as_array().expect("registrations");
    assert_eq!(registrations.len(), 3);
    let steam_entry = registrations
        .iter()
        .find(|entry| path_of(&entry["path"]) == cli.libraries.steam)
        .expect("steam registration");
    assert_eq!(steam_entry["source"], "manual");
}

#[test]
fn register_follows_the_selected_state_directory() {
    let cli = Cli::new();
    let profile = tempfile::tempdir().expect("profile");
    let state_dir = profile.path().join("profile");
    let state_dir_text = state_dir.to_str().expect("state dir");

    let report = cli.json(&[
        "--state-dir",
        state_dir_text,
        "game",
        "detect",
        "--register",
        "--json",
    ]);

    assert_eq!(installations(&report).len(), 3);
    assert!(state_dir.join("state.sqlite").is_file());
    assert!(!cli.state().exists(), "the default profile stays untouched");
    let listed = cli.json(&["--state-dir", state_dir_text, "game", "list", "--json"]);
    assert_eq!(listed.as_array().expect("registrations").len(), 3);
}

#[test]
fn add_without_a_path_needs_exactly_one_confirmed_installation() {
    let cli = Cli::new();

    let ambiguous = cli.run(&["game", "add", "cyberpunk-2077", "--json"]);
    let balatro = cli.run(&["game", "add", "balatro", "--json"]);

    assert!(!ambiguous.status.success());
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("multiple installations"));
    assert!(
        !balatro.status.success(),
        "a refused folder is not registered"
    );
    assert!(String::from_utf8_lossy(&balatro.stderr).contains("balatro not found"));
    assert!(!cli.state().exists());
}
