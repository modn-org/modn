#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "support/module_catalog.rs"]
mod module_catalog;

use module_catalog::{identity_lua, ModuleCatalog};
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn cli(data: &Path, args: &[&str]) -> Output {
    Command::new(main_bucket::modn_binary())
        .env("LOCALAPPDATA", data)
        .env_remove("MODN_GAME_MODULES_DIR")
        .args(args)
        .output()
        .expect("run modn")
}

fn success(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("JSON output")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn sync(data: &Path, catalog: &ModuleCatalog) -> Value {
    success(&cli(
        data,
        &[
            "--json",
            "bucket",
            "sync",
            "community",
            "--source",
            catalog.repo().to_str().expect("repository path"),
            "--revision",
            "HEAD",
        ],
    ))
}

fn trust(data: &Path, catalog: &ModuleCatalog) -> Value {
    success(&cli(
        data,
        &[
            "--json",
            "bucket",
            "trust-modules",
            "community",
            "--yes",
            "--fingerprint",
            catalog.key_id(),
        ],
    ))
}

fn release_entry<'a>(listed: &'a Value, game: &str, version: &str) -> &'a Value {
    listed
        .as_array()
        .expect("module list array")
        .iter()
        .find(|entry| entry["module"]["game_id"] == game && entry["module"]["version"] == version)
        .expect("retained release")
}

#[test]
fn trust_requires_acknowledgement_and_matching_fingerprint_before_writing() {
    let data = tempfile::tempdir().expect("data directory");
    main_bucket::seed(data.path());
    let catalog = ModuleCatalog::new("community");
    catalog.write_module("fixture-game", "1.0.0", &identity_lua("fixture-game"));
    catalog.sign_and_commit("module v1");
    let synced = sync(data.path(), &catalog);
    assert_eq!(synced["modules"]["ignored"][0]["game_id"], "fixture-game");
    assert_eq!(
        synced["modules"]["ignored"][0]["reason"],
        "untrusted_source"
    );

    let registration = data
        .path()
        .join("modn/buckets/registrations/community.json");
    let before = fs::read(&registration).expect("registration bytes");

    let denied = cli(data.path(), &["bucket", "trust-modules", "community"]);
    assert!(!denied.status.success());
    let message = stderr(&denied);
    assert!(message.contains(catalog.key_id()), "{message}");
    assert!(message.contains("--yes"), "{message}");
    assert_eq!(
        fs::read(&registration).expect("registration"),
        before,
        "refused trust must not write"
    );

    let wrong = cli(
        data.path(),
        &[
            "bucket",
            "trust-modules",
            "community",
            "--yes",
            "--fingerprint",
            "0000000000000000",
        ],
    );
    assert!(!wrong.status.success());
    assert!(stderr(&wrong).contains("signer fingerprint mismatch"));
    assert_eq!(
        fs::read(&registration).expect("registration"),
        before,
        "fingerprint mismatch must not write"
    );

    let trusted = cli(
        data.path(),
        &[
            "bucket",
            "trust-modules",
            "community",
            "--yes",
            "--fingerprint",
            catalog.key_id(),
        ],
    );
    assert!(trusted.status.success(), "{}", stderr(&trusted));
    assert_eq!(
        stdout(&trusted).trim(),
        format!(
            "Trusted signer {} for bucket community; modules from this bucket are executable policy.",
            catalog.key_id()
        )
    );
    let stored: Value =
        serde_json::from_slice(&fs::read(&registration).expect("registration")).expect("JSON");
    assert_eq!(stored["module_trust"]["signer_key_id"], catalog.key_id());

    let trusted_json = success(&cli(
        data.path(),
        &["--json", "bucket", "trust-modules", "community", "--yes"],
    ));
    assert_eq!(trusted_json["bucket"], "community");
    assert_eq!(trusted_json["signer_key_id"], catalog.key_id());
    assert_eq!(trusted_json["imported"][0]["game_id"], "fixture-game");
    assert_eq!(trusted_json["imported"][0]["version"], "1.0.0");
}

#[test]
fn main_and_local_folder_buckets_cannot_be_module_sources() {
    let data = tempfile::tempdir().expect("data directory");
    main_bucket::seed(data.path());

    let main = cli(data.path(), &["bucket", "trust-modules", "main", "--yes"]);
    assert!(!main.status.success());
    assert!(
        stderr(&main).contains("official main bucket is always a module source"),
        "{}",
        stderr(&main)
    );

    let local = data.path().join("local-bucket");
    fs::create_dir_all(&local).expect("local bucket");
    fs::write(
        local.join("bucket.json"),
        r#"{"schema_version":1,"name":"local","title":"Local fixture"}"#,
    )
    .expect("bucket metadata");
    success(&cli(
        data.path(),
        &[
            "--json",
            "bucket",
            "add",
            "local",
            local.to_str().expect("path"),
        ],
    ));
    let refused = cli(data.path(), &["bucket", "trust-modules", "local", "--yes"]);
    assert!(!refused.status.success());
    assert!(
        stderr(&refused).contains("local folder buckets cannot be module sources"),
        "{}",
        stderr(&refused)
    );
}

#[test]
fn pin_blocks_update_and_rollback_and_list_marks_retained_releases() {
    let data = tempfile::tempdir().expect("data directory");
    main_bucket::seed(data.path());
    let catalog = ModuleCatalog::new("community");
    catalog.write_module("fixture-game", "1.0.0", &identity_lua("fixture-game"));
    catalog.sign_and_commit("module v1");
    sync(data.path(), &catalog);
    trust(data.path(), &catalog);

    let listed = success(&cli(data.path(), &["--json", "module", "list"]));
    let first = release_entry(&listed, "fixture-game", "1.0.0");
    assert_eq!(first["active"], true);
    assert_eq!(first["pinned"], false);
    assert_eq!(first["source_trusted"], true);
    assert_eq!(first["source"]["bucket"], "community");
    assert_eq!(
        first["sha256"].as_str().expect("digest").len(),
        64,
        "release digest"
    );

    let pinned = success(&cli(
        data.path(),
        &["--json", "module", "pin", "fixture-game"],
    ));
    assert_eq!(pinned["game_id"], "fixture-game");
    assert_eq!(pinned["pinned"]["bucket"], "community");
    assert_eq!(pinned["pinned"]["version"], "1.0.0");
    assert_eq!(pinned["pinned"]["sha256"], first["sha256"]);

    catalog.write_module("fixture-game", "2.0.0", &identity_lua("fixture-game"));
    catalog.sign_and_commit("module v2");
    let blocked = success(&cli(
        data.path(),
        &["--json", "module", "update", "fixture-game"],
    ));
    assert_eq!(blocked["game_id"], "fixture-game");
    assert_eq!(blocked["from"], "1.0.0");
    assert_eq!(blocked["to"], "1.0.0");
    assert_eq!(blocked["outcome"], "pinned");

    let unpinned = success(&cli(
        data.path(),
        &["--json", "module", "unpin", "fixture-game"],
    ));
    assert_eq!(unpinned["pinned"], Value::Null);
    let updated = success(&cli(
        data.path(),
        &["--json", "module", "update", "fixture-game"],
    ));
    assert_eq!(updated["from"], "1.0.0");
    assert_eq!(updated["to"], "2.0.0");
    assert_eq!(updated["outcome"], "imported");

    let listed = success(&cli(data.path(), &["--json", "module", "list"]));
    assert_eq!(listed.as_array().expect("list array").len(), 2);
    assert_eq!(
        release_entry(&listed, "fixture-game", "1.0.0")["active"],
        false
    );
    assert_eq!(
        release_entry(&listed, "fixture-game", "2.0.0")["active"],
        true
    );

    let rolled = success(&cli(
        data.path(),
        &[
            "--json",
            "module",
            "rollback",
            "fixture-game",
            "--to",
            "1.0.0",
        ],
    ));
    assert_eq!(rolled["from"], "2.0.0");
    assert_eq!(rolled["to"], "1.0.0");
    let plain = cli(data.path(), &["module", "list"]);
    assert!(plain.status.success(), "{}", stderr(&plain));
    let text = stdout(&plain);
    assert!(text.contains("fixture-game v1.0.0"), "{text}");
    assert!(text.contains("fixture-game v2.0.0"), "{text}");
    assert!(text.contains("[active]"), "{text}");

    success(&cli(
        data.path(),
        &["--json", "module", "pin", "fixture-game"],
    ));
    let refused = cli(
        data.path(),
        &["module", "rollback", "fixture-game", "--to", "2.0.0"],
    );
    assert!(!refused.status.success());
    assert!(
        stderr(&refused).contains("modn module unpin fixture-game"),
        "{}",
        stderr(&refused)
    );
    success(&cli(
        data.path(),
        &["--json", "module", "unpin", "fixture-game"],
    ));
}

#[test]
fn local_source_update_and_untrusted_bucket_use_fail_with_hints() {
    let data = tempfile::tempdir().expect("data directory");
    main_bucket::seed(data.path());

    let module = data.path().join("local-module");
    fs::create_dir_all(&module).expect("local module");
    fs::write(
        module.join("module.json"),
        r#"{"game_id":"local-game","version":"1.0.0","api_version":1,"entrypoint":"module.lua"}"#,
    )
    .expect("manifest");
    fs::write(module.join("module.lua"), identity_lua("local-game")).expect("Lua");
    success(&cli(
        data.path(),
        &[
            "--json",
            "module",
            "register",
            module.to_str().expect("path"),
            "--trust",
        ],
    ));
    let local_update = cli(data.path(), &["module", "update", "local-game"]);
    assert!(!local_update.status.success());
    assert!(
        stderr(&local_update).contains("modn module register"),
        "{}",
        stderr(&local_update)
    );

    let catalog = ModuleCatalog::new("community");
    catalog.write_module("fixture-game", "1.0.0", &identity_lua("fixture-game"));
    catalog.sign_and_commit("module v1");
    sync(data.path(), &catalog);
    trust(data.path(), &catalog);
    let removed = success(&cli(
        data.path(),
        &["--json", "bucket", "untrust-modules", "community"],
    ));
    assert_eq!(removed["bucket"], "community");
    assert_eq!(removed["module_trust"], "removed");

    let listed = success(&cli(data.path(), &["--json", "module", "list"]));
    let untrusted = listed
        .as_array()
        .expect("list array")
        .iter()
        .find(|entry| entry["module"]["game_id"] == "fixture-game")
        .expect("retained release");
    assert_eq!(untrusted["source_trusted"], false);

    let plain = cli(data.path(), &["module", "list"]);
    assert!(plain.status.success(), "{}", stderr(&plain));
    let text = stdout(&plain);
    assert!(text.contains("warning:"), "{text}");
    assert!(
        text.contains("modn bucket trust-modules community"),
        "{text}"
    );

    let update = cli(data.path(), &["module", "update", "fixture-game"]);
    assert!(!update.status.success());
    assert!(
        stderr(&update).contains("modn bucket trust-modules community"),
        "{}",
        stderr(&update)
    );

    let game = data.path().join("game");
    fs::create_dir_all(&game).expect("game directory");
    let doctor = cli(
        data.path(),
        &[
            "--game",
            "fixture-game",
            "doctor",
            "--path",
            game.to_str().expect("path"),
        ],
    );
    assert!(!doctor.status.success());
    assert!(
        stderr(&doctor).contains("modn bucket trust-modules community"),
        "{}",
        stderr(&doctor)
    );
}

#[test]
fn sync_reports_imported_modules_for_trusted_and_ignored_for_untrusted() {
    let data = tempfile::tempdir().expect("data directory");
    main_bucket::seed(data.path());
    let catalog = ModuleCatalog::new("community");
    catalog.write_module("fixture-game", "1.0.0", &identity_lua("fixture-game"));
    catalog.sign_and_commit("module v1");
    let ignored = sync(data.path(), &catalog);
    assert_eq!(ignored["modules"]["ignored"][0]["game_id"], "fixture-game");
    assert!(ignored["modules"]["imported"]
        .as_array()
        .expect("imported array")
        .is_empty());

    trust(data.path(), &catalog);
    catalog.write_module("fixture-game", "2.0.0", &identity_lua("fixture-game"));
    catalog.sign_and_commit("module v2");
    let imported = sync(data.path(), &catalog);
    assert!(imported["modules"]["ignored"]
        .as_array()
        .expect("ignored array")
        .is_empty());
    assert_eq!(
        imported["modules"]["imported"][0]["game_id"],
        "fixture-game"
    );
    assert_eq!(imported["modules"]["imported"][0]["version"], "2.0.0");
    assert_eq!(imported["modules"]["imported"][0]["outcome"], "imported");

    let plain = cli(data.path(), &["bucket", "sync", "community"]);
    assert!(plain.status.success(), "{}", stderr(&plain));
    let text = stdout(&plain);
    assert!(text.contains("Synced bucket community"), "{text}");
    assert!(
        text.contains("Game module fixture-game v2.0.0: unchanged"),
        "{text}"
    );
}
