//! One retained release must get one trust answer from every reader.
//!
//! A catalog signer change ends module trust. Re-trusting the bucket after the
//! change trusts the new signer, not the release the bucket already retained.
//! `module list` and game policy resolution must agree that the retained
//! release is refused, before and after the signer change.

#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "support/module_catalog.rs"]
mod module_catalog;

use module_catalog::{policy_lua, ModuleCatalog};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Fixture {
    _root: tempfile::TempDir,
    localappdata: PathBuf,
    appdata: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("disposable fixture root");
        let localappdata = root.path().join("localappdata");
        let appdata = root.path().join("appdata");
        fs::create_dir_all(&localappdata).expect("disposable localappdata");
        fs::create_dir_all(&appdata).expect("disposable appdata");
        main_bucket::seed(&localappdata);
        Self {
            _root: root,
            localappdata,
            appdata,
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(main_bucket::modn_binary())
            .env("LOCALAPPDATA", &self.localappdata)
            .env("APPDATA", &self.appdata)
            .env("MODN_APPDATA", &self.appdata)
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

    fn game_directory(&self) -> PathBuf {
        let game = self.localappdata.join("game-fixture");
        fs::create_dir_all(&game).expect("fixture game directory");
        fs::write(game.join("marker.bin"), b"marker").expect("policy marker");
        game
    }

    fn doctor(&self, game: &Path) -> Output {
        self.run(&[
            "--json",
            "--game",
            "fixture-game",
            "doctor",
            "--path",
            game.to_str().expect("fixture game path"),
        ])
    }
}

fn release<'a>(listed: &'a Value, version: &str) -> Option<&'a Value> {
    listed.as_array()?.iter().find(|entry| {
        entry["module"]["game_id"] == "fixture-game" && entry["module"]["version"] == version
    })
}

fn bucket_entry<'a>(listed: &'a Value, name: &str) -> Option<&'a Value> {
    listed
        .as_array()?
        .iter()
        .find(|entry| entry["name"] == name)
}

/// The two answers under test, read from the same workspace.
struct Answers {
    list_trusted: bool,
    policy_refused: bool,
}

fn answers(fixture: &Fixture, game: &Path) -> Answers {
    let listed = fixture.success(&["--json", "module", "list"]);
    let source_trusted = release(&listed, "1.0.0").expect("retained release")["source_trusted"]
        .as_bool()
        .expect("source_trusted is a boolean");
    let doctor = fixture.doctor(game);
    Answers {
        list_trusted: source_trusted,
        policy_refused: !doctor.status.success(),
    }
}

#[test]
fn module_list_and_policy_agree_after_a_catalog_signer_change() {
    let fixture = Fixture::new();
    let game = fixture.game_directory();
    let mut catalog = ModuleCatalog::new("third");
    catalog.write_module(
        "fixture-game",
        "1.0.0",
        &policy_lua("fixture-game", "marker.bin"),
    );
    catalog.sign_and_commit("fixture module under the first signer");
    let first_signer = catalog.key_id().to_owned();

    fixture.success(&[
        "--json",
        "bucket",
        "sync",
        "third",
        "--source",
        catalog.repo().to_str().expect("repository path"),
        "--revision",
        "HEAD",
    ]);
    let trusted = fixture.success(&[
        "--json",
        "bucket",
        "trust-modules",
        "third",
        "--yes",
        "--fingerprint",
        &first_signer,
    ]);
    assert_eq!(trusted["signer_key_id"], first_signer);

    // Control: while the catalog signer still matches the trust record, both
    // readers call the retained release a trusted module source.
    let before = answers(&fixture, &game);
    assert!(
        before.list_trusted,
        "module list must trust the release before the signer change"
    );
    assert!(
        !before.policy_refused,
        "policy resolution must accept the release before the signer change"
    );
    assert_eq!(
        before.list_trusted, !before.policy_refused,
        "module list and policy must agree before the signer change"
    );

    // A plain signer change publishes a new catalog key. The documented way to
    // move the pin is to remove the bucket and register it again; no rotation
    // statement vouches for the old key.
    fixture.success(&["--json", "bucket", "remove", "third"]);
    catalog.change_signer();
    catalog.sign_and_commit("fixture module under the second signer");
    let second_signer = catalog.key_id().to_owned();
    assert_ne!(first_signer, second_signer);

    fixture.success(&[
        "--json",
        "bucket",
        "sync",
        "third",
        "--source",
        catalog.repo().to_str().expect("repository path"),
        "--revision",
        "HEAD",
    ]);
    let buckets = fixture.success(&["--json", "bucket", "list"]);
    assert_eq!(
        bucket_entry(&buckets, "third").expect("re-registered bucket")["signer_key_id"],
        second_signer
    );
    let re_trusted = fixture.success(&[
        "--json",
        "bucket",
        "trust-modules",
        "third",
        "--yes",
        "--fingerprint",
        &second_signer,
    ]);
    assert_eq!(re_trusted["signer_key_id"], second_signer);

    // The release is retained from the first signer, and the unchanged import
    // did not rewrite its recorded source.
    let listed = fixture.success(&["--json", "module", "list"]);
    let retained = release(&listed, "1.0.0").expect("retained release");
    assert_eq!(retained["source"]["bucket"], "third");
    assert_eq!(retained["source"]["signer_key_id"], first_signer);
    assert_eq!(retained["active"], true);

    let after = answers(&fixture, &game);
    assert!(
        !after.list_trusted,
        "module list must refuse the release after the signer change: {listed}"
    );
    assert!(
        after.policy_refused,
        "policy resolution must refuse the release after the signer change"
    );
    assert_eq!(
        after.list_trusted, !after.policy_refused,
        "module list and policy must give the same answer after the signer change"
    );

    let doctor = fixture.doctor(&game);
    let refused = String::from_utf8_lossy(&doctor.stderr);
    assert!(
        refused.contains("not a trusted module source"),
        "policy refusal must name the untrusted module source: {refused}"
    );
    assert!(
        refused.contains("modn bucket trust-modules third"),
        "policy refusal must name the trust command: {refused}"
    );
}
