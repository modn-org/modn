//! Milestone 5 exit gate: a trusted signed Git catalog supplies a module that
//! can be pinned, updated and rolled back; untrusted sources are ignored and
//! refused at use; the official module keeps priority.

#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "support/module_catalog.rs"]
mod module_catalog;

use module_catalog::{identity_lua, ModuleCatalog};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Disposable manager data, appdata and game directories.
struct Fixture {
    _root: tempfile::TempDir,
    data: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("disposable fixture root");
        let data = root.path().join("localappdata");
        fs::create_dir_all(data.join("appdata")).expect("disposable appdata");
        main_bucket::seed(&data);
        Self { _root: root, data }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(main_bucket::modn_binary())
            .env("LOCALAPPDATA", &self.data)
            .env("APPDATA", self.data.join("appdata"))
            .env_remove("MODN_GAME_MODULES_DIR")
            .env_remove("MODN_MAIN_BUCKET_SOURCE")
            .env_remove("MODN_TEST_MAIN_BUCKET_PUBLIC_KEY")
            .args(args)
            .output()
            .expect("run modn")
    }

    fn success(&self, args: &[&str]) -> Value {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "modn {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "modn {args:?} did not print JSON ({error}): {}",
                String::from_utf8_lossy(&output.stdout)
            )
        })
    }

    fn failure(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(
            !output.status.success(),
            "modn {args:?} unexpectedly succeeded: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        String::from_utf8_lossy(&output.stderr).into_owned()
    }

    fn sync(&self, name: &str, catalog: &ModuleCatalog) -> Value {
        self.success(&[
            "--json",
            "bucket",
            "sync",
            name,
            "--source",
            catalog.repo().to_str().expect("repository path"),
            "--revision",
            "HEAD",
        ])
    }
}

fn module_report<'a>(synced: &'a Value, section: &str, game: &str) -> Option<&'a Value> {
    synced["modules"][section]
        .as_array()?
        .iter()
        .find(|entry| entry["game_id"] == game)
}

fn release<'a>(listed: &'a Value, game: &str, version: &str) -> Option<&'a Value> {
    listed
        .as_array()?
        .iter()
        .find(|entry| entry["module"]["game_id"] == game && entry["module"]["version"] == version)
}

fn active_version<'a>(listed: &'a Value, game: &str) -> Option<&'a str> {
    listed.as_array()?.iter().find_map(|entry| {
        (entry["module"]["game_id"] == game && entry["active"] == true)
            .then(|| entry["module"]["version"].as_str())
            .flatten()
    })
}

/// Every path and file byte below `root`, so a rollback can be proven inert.
fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, directory: &Path, entries: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(directory).expect("tree entries") {
            let entry = entry.expect("tree entry");
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .expect("tree path")
                .to_string_lossy()
                .replace('\\', "/");
            if entry.file_type().expect("tree type").is_dir() {
                entries.insert(format!("{relative}/"), Vec::new());
                walk(root, &path, entries);
            } else {
                entries.insert(relative, fs::read(&path).expect("tree file"));
            }
        }
    }
    let mut entries = BTreeMap::new();
    walk(root, root, &mut entries);
    entries
}

#[test]
fn milestone_five_gate_trusts_pins_updates_rolls_back_and_keeps_official_priority() {
    let fixture = Fixture::new();
    let third = ModuleCatalog::new("third");
    let third_repo = third.repo().to_str().expect("repository path").to_owned();

    // Step 1: a signed Git catalog is registered by sync, but its module is
    // ignored until the user trusts the bucket for modules.
    third.write_module("fixture-game", "1.0.0", &identity_lua("fixture-game"));
    third.sign_and_commit("third fixture module v1");
    let synced = fixture.sync("third", &third);
    assert_eq!(
        module_report(&synced, "ignored", "fixture-game").expect("ignored report")["reason"],
        "untrusted_source"
    );
    assert!(
        module_report(&synced, "imported", "fixture-game").is_none(),
        "untrusted module must not be imported"
    );
    let buckets = fixture.success(&["--json", "bucket", "list"]);
    assert!(
        buckets
            .as_array()
            .expect("bucket list")
            .iter()
            .any(|bucket| { bucket["name"] == "third" && bucket["module_trusted"] == false }),
        "{buckets}"
    );
    let listed = fixture.success(&["--json", "module", "list"]);
    assert!(
        release(&listed, "fixture-game", "1.0.0").is_none(),
        "no release exists before trust: {listed}"
    );

    // Step 2: trust needs acknowledgement and a matching fingerprint, then
    // imports version 1.0.0 as the active release.
    let registration = fixture.data.join("modn/buckets/registrations/third.json");
    let before = fs::read(&registration).expect("registration bytes");
    let denied = fixture.failure(&["bucket", "trust-modules", "third"]);
    assert!(denied.contains(third.key_id()), "{denied}");
    assert!(denied.contains("--yes"), "{denied}");
    assert_eq!(
        fs::read(&registration).expect("registration"),
        before,
        "refused trust must not write"
    );
    let trusted = fixture.success(&[
        "--json",
        "bucket",
        "trust-modules",
        "third",
        "--yes",
        "--fingerprint",
        third.key_id(),
    ]);
    assert_eq!(trusted["bucket"], "third");
    assert_eq!(trusted["signer_key_id"], third.key_id());
    assert_eq!(trusted["imported"][0]["game_id"], "fixture-game");
    assert_eq!(trusted["imported"][0]["version"], "1.0.0");
    assert_eq!(trusted["imported"][0]["outcome"], "imported");
    let listed = fixture.success(&["--json", "module", "list"]);
    assert_eq!(active_version(&listed, "fixture-game"), Some("1.0.0"));
    let first = release(&listed, "fixture-game", "1.0.0").expect("release 1.0.0");
    assert_eq!(first["source"]["bucket"], "third");
    assert_eq!(first["source_trusted"], true);

    // Step 3: pinning keeps 1.0.0 active while a newer release is published.
    let pinned = fixture.success(&["--json", "module", "pin", "fixture-game"]);
    assert_eq!(pinned["game_id"], "fixture-game");
    assert_eq!(pinned["pinned"]["bucket"], "third");
    assert_eq!(pinned["pinned"]["version"], "1.0.0");
    third.write_module("fixture-game", "2.0.0", &identity_lua("fixture-game"));
    third.sign_and_commit("third fixture module v2");
    let blocked = fixture.success(&["--json", "module", "update", "fixture-game"]);
    assert_eq!(blocked["game_id"], "fixture-game");
    assert_eq!(blocked["from"], "1.0.0");
    assert_eq!(blocked["to"], "1.0.0");
    assert_eq!(blocked["outcome"], "pinned");
    let listed = fixture.success(&["--json", "module", "list"]);
    assert_eq!(active_version(&listed, "fixture-game"), Some("1.0.0"));

    // Step 4: unpinning lets the update switch to 2.0.0 and retains 1.0.0.
    let unpinned = fixture.success(&["--json", "module", "unpin", "fixture-game"]);
    assert_eq!(unpinned["pinned"], Value::Null);
    let updated = fixture.success(&["--json", "module", "update", "fixture-game"]);
    assert_eq!(updated["from"], "1.0.0");
    assert_eq!(updated["to"], "2.0.0");
    assert_eq!(updated["outcome"], "imported");
    let listed = fixture.success(&["--json", "module", "list"]);
    assert_eq!(active_version(&listed, "fixture-game"), Some("2.0.0"));
    assert_eq!(
        release(&listed, "fixture-game", "1.0.0").expect("retained 1.0.0")["active"],
        false
    );

    // Step 5: rollback moves only the active pointer; game files stay intact.
    let fixture_game = fixture.data.join("game-fixture");
    fs::create_dir_all(&fixture_game).expect("fixture game directory");
    fs::write(fixture_game.join("untouched.bin"), b"do not touch").expect("game marker");
    let before = tree(&fixture_game);
    let rolled = fixture.success(&["--json", "module", "rollback", "fixture-game"]);
    assert_eq!(rolled["game_id"], "fixture-game");
    assert_eq!(rolled["from"], "2.0.0");
    assert_eq!(rolled["to"], "1.0.0");
    assert_eq!(
        tree(&fixture_game),
        before,
        "rollback must not touch game files"
    );
    let listed = fixture.success(&["--json", "module", "list"]);
    assert_eq!(active_version(&listed, "fixture-game"), Some("1.0.0"));

    // Step 6: an untrusted catalog imports nothing and its Lua never runs.
    let other = ModuleCatalog::new("other");
    other.write_module("other-game", "1.0.0", "error('OTHER-GAME MODULE EXECUTED')");
    other.sign_and_commit("other module v1");
    let other_sync = fixture.sync("other", &other);
    assert_eq!(
        module_report(&other_sync, "ignored", "other-game").expect("ignored report")["reason"],
        "untrusted_source"
    );
    assert!(module_report(&other_sync, "imported", "other-game").is_none());
    let listed = fixture.success(&["--json", "module", "list"]);
    assert!(
        listed
            .as_array()
            .expect("module list")
            .iter()
            .all(|entry| entry["module"]["game_id"] != "other-game"),
        "{listed}"
    );
    let other_game = fixture.data.join("game-other");
    fs::create_dir_all(&other_game).expect("other game directory");
    fs::write(other_game.join("marker.bin"), b"marker").expect("other marker");
    let refused = fixture.failure(&[
        "--game",
        "other-game",
        "doctor",
        "--path",
        other_game.to_str().expect("other game path"),
    ]);
    assert!(
        !refused.contains("OTHER-GAME MODULE EXECUTED"),
        "untrusted Lua must not run: {refused}"
    );

    // Untrusting a bucket keeps retained releases but refuses them at use.
    let untrusted = fixture.success(&["--json", "bucket", "untrust-modules", "third"]);
    assert_eq!(untrusted["bucket"], "third");
    assert_eq!(untrusted["module_trust"], "removed");
    let listed = fixture.success(&["--json", "module", "list"]);
    assert_eq!(
        release(&listed, "fixture-game", "1.0.0").expect("retained release")["source_trusted"],
        false
    );
    let refused = fixture.failure(&[
        "--game",
        "fixture-game",
        "doctor",
        "--path",
        fixture_game.to_str().expect("fixture game path"),
    ]);
    assert!(
        refused.contains("modn bucket trust-modules third"),
        "use must name the trust command: {refused}"
    );

    // Step 7: a trusted catalog module for an official game stays hidden; the
    // official module is used and doctor reports the priority override.
    let re_trusted = fixture.success(&[
        "--json",
        "bucket",
        "trust-modules",
        "third",
        "--yes",
        "--fingerprint",
        third.key_id(),
    ]);
    assert_eq!(re_trusted["signer_key_id"], third.key_id());
    third.write_module(
        "cyberpunk-2077",
        "9.9.9",
        "error('THIRD-PARTY CYBERPUNK MODULE EXECUTED')",
    );
    third.sign_and_commit("third official-priority module");
    let synced = fixture.success(&[
        "--json",
        "bucket",
        "sync",
        "third",
        "--source",
        &third_repo,
        "--revision",
        "HEAD",
    ]);
    assert_eq!(
        module_report(&synced, "ignored", "cyberpunk-2077").expect("official priority report")
            ["reason"],
        "official_priority"
    );
    assert!(module_report(&synced, "imported", "cyberpunk-2077").is_none());
    let listed = fixture.success(&["--json", "module", "list"]);
    assert!(
        listed
            .as_array()
            .expect("module list")
            .iter()
            .all(|entry| entry["module"]["game_id"] != "cyberpunk-2077"),
        "{listed}"
    );
    let cyberpunk = fixture.data.join("game-cyberpunk");
    fs::create_dir_all(cyberpunk.join("bin/x64")).expect("cyberpunk installation");
    fs::write(cyberpunk.join("bin/x64/Cyberpunk2077.exe"), b"").expect("official marker");
    let doctor = fixture.success(&[
        "--json",
        "--game",
        "cyberpunk-2077",
        "doctor",
        "--path",
        cyberpunk.to_str().expect("cyberpunk path"),
    ]);
    let hidden = doctor["diagnostics"]
        .as_array()
        .expect("doctor diagnostics")
        .iter()
        .find(|diagnostic| diagnostic["code"] == "MODULE_HIDDEN_BY_OFFICIAL")
        .unwrap_or_else(|| panic!("missing MODULE_HIDDEN_BY_OFFICIAL: {doctor}"));
    assert!(
        hidden["message"]
            .as_str()
            .expect("hidden message")
            .contains("official main bucket takes priority"),
        "{hidden}"
    );
    assert!(
        !doctor
            .to_string()
            .contains("THIRD-PARTY CYBERPUNK MODULE EXECUTED"),
        "third-party Lua must not run"
    );
}
