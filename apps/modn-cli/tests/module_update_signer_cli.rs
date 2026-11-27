//! `modn module update` must refuse a release whose recorded signer the
//! bucket's trusted signer no longer authorizes, and the recovery it names must
//! restore updates.

#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "support/module_catalog.rs"]
mod module_catalog;

use module_catalog::{identity_lua, ModuleCatalog};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
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

    fn sync(&self, catalog: &ModuleCatalog) -> Value {
        let repository = catalog.repo().to_str().expect("repository path");
        self.success(&[
            "--json",
            "bucket",
            "sync",
            "third",
            "--source",
            repository,
            "--revision",
            "HEAD",
        ])
    }

    fn trust(&self, signer: &str) -> Value {
        self.success(&[
            "--json",
            "bucket",
            "trust-modules",
            "third",
            "--yes",
            "--fingerprint",
            signer,
        ])
    }
}

fn active_release(listed: &Value) -> &Value {
    listed
        .as_array()
        .expect("module list array")
        .iter()
        .find(|entry| entry["module"]["game_id"] == "fixture-game" && entry["active"] == true)
        .expect("active fixture-game release")
}

#[test]
fn module_update_refuses_a_release_signer_the_bucket_no_longer_authorizes() {
    let fixture = Fixture::new();
    let mut catalog = ModuleCatalog::new("third");
    catalog.write_module("fixture-game", "1.0.0", &identity_lua("fixture-game"));
    catalog.sign_and_commit("module under the first signer");
    let first_signer = catalog.key_id().to_owned();
    fixture.sync(&catalog);
    fixture.trust(&first_signer);

    // Control: while the trusted signer matches the active release, update runs.
    let updated = fixture.success(&["--json", "module", "update", "fixture-game"]);
    assert_eq!(updated["from"], "1.0.0");
    assert_eq!(updated["to"], "1.0.0");

    // A plain signer change: remove and re-register the bucket, publish the new
    // key, then trust it. The retained release still records the first signer.
    fixture.success(&["--json", "bucket", "remove", "third"]);
    catalog.change_signer();
    catalog.sign_and_commit("module under the second signer");
    let second_signer = catalog.key_id().to_owned();
    assert_ne!(first_signer, second_signer);
    fixture.sync(&catalog);
    let trusted = fixture.trust(&second_signer);
    assert_eq!(trusted["signer_key_id"], second_signer);
    let listed = fixture.success(&["--json", "module", "list"]);
    let retained = active_release(&listed);
    assert_eq!(retained["source"]["bucket"], "third");
    assert_eq!(retained["source"]["signer_key_id"], first_signer);

    // The refusal names the bucket and both recovery commands.
    let refused = fixture.run(&["module", "update", "fixture-game"]);
    let message = String::from_utf8_lossy(&refused.stderr);
    assert!(
        !refused.status.success(),
        "module update must refuse a release signer the bucket no longer authorizes: {message}"
    );
    assert!(
        message.contains("game module fixture-game comes from bucket third"),
        "refusal must name the module and bucket: {message}"
    );
    assert!(
        message.contains("modn bucket trust-modules third"),
        "refusal must name the trust command: {message}"
    );
    assert!(
        message.contains("modn module remove fixture-game"),
        "refusal must name the remove command: {message}"
    );
    assert!(
        message.contains("is not authorized by the trusted signer"),
        "refusal must name the signer change: {message}"
    );
    let remove_at = message
        .find("modn module remove fixture-game")
        .expect("refusal names the remove command");
    let trust_at = message
        .find("modn bucket trust-modules third")
        .expect("refusal names the trust command");
    assert!(
        remove_at < trust_at,
        "after a signer change the refusal must remove the module first: {message}"
    );

    // `module list` carries the same reason, order and commands as a warning.
    let listed_text = fixture.run(&["module", "list"]);
    let warning = String::from_utf8_lossy(&listed_text.stdout);
    assert!(
        listed_text.status.success(),
        "module list failed: {}",
        String::from_utf8_lossy(&listed_text.stderr)
    );
    assert!(
        warning.contains("warning: bucket third is not a trusted module source"),
        "list warning must name the untrusted bucket: {warning}"
    );
    assert!(
        warning.contains("is not authorized by the trusted signer"),
        "list warning must name the signer change: {warning}"
    );
    assert!(
        warning.contains(
            "Run `modn module remove fixture-game` first, then `modn bucket trust-modules third`"
        ),
        "list warning must remove the module before trusting the bucket: {warning}"
    );

    // The recovery the refusal names: remove the retained release, then trust
    // the bucket again so the catalog module is imported under the new signer.
    let removed = fixture.success(&["--json", "module", "remove", "fixture-game"]);
    assert_eq!(removed["removed"], "fixture-game");
    let reimported = fixture.trust(&second_signer);
    assert_eq!(reimported["imported"][0]["game_id"], "fixture-game");
    assert_eq!(reimported["imported"][0]["outcome"], "imported");
    let listed = fixture.success(&["--json", "module", "list"]);
    let recovered = active_release(&listed);
    assert_eq!(recovered["source"]["signer_key_id"], second_signer);
    assert_eq!(recovered["source_trusted"], true);

    let updated = fixture.success(&["--json", "module", "update", "fixture-game"]);
    assert_eq!(updated["from"], "1.0.0");
    assert_eq!(updated["to"], "1.0.0");
}

#[test]
fn module_update_and_list_never_trust_an_unregistered_bucket() {
    let fixture = Fixture::new();
    let catalog = ModuleCatalog::new("third");
    catalog.write_module("fixture-game", "1.0.0", &identity_lua("fixture-game"));
    catalog.sign_and_commit("module before the bucket removal");
    fixture.sync(&catalog);
    fixture.trust(catalog.key_id());

    // The retained release keeps naming bucket third after the bucket is gone.
    fixture.success(&["--json", "bucket", "remove", "third"]);

    let listed_text = fixture.run(&["module", "list"]);
    let warning = String::from_utf8_lossy(&listed_text.stdout);
    assert!(
        listed_text.status.success(),
        "module list failed: {}",
        String::from_utf8_lossy(&listed_text.stderr)
    );
    assert!(
        warning.contains("bucket third is not registered"),
        "list warning must name the unregistered bucket: {warning}"
    );
    assert!(
        warning.contains("modn module remove fixture-game"),
        "list warning must name the remove command: {warning}"
    );
    assert!(
        !warning.contains("trust-modules"),
        "an unregistered bucket cannot be trusted with trust-modules: {warning}"
    );

    let refused = fixture.run(&["module", "update", "fixture-game"]);
    let message = String::from_utf8_lossy(&refused.stderr);
    assert!(
        !refused.status.success(),
        "module update must refuse a release from an unregistered bucket: {message}"
    );
    assert!(
        message.contains("bucket third is not registered"),
        "refusal must name the unregistered bucket: {message}"
    );
    assert!(
        message.contains("modn module remove fixture-game"),
        "refusal must name the remove command: {message}"
    );
    assert!(
        !message.contains("trust-modules"),
        "an unregistered bucket cannot be trusted with trust-modules: {message}"
    );
}
