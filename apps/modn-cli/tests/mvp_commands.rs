#[path = "support/main_bucket.rs"]
mod main_bucket;

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn cli(data: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_modn"))
        .env("LOCALAPPDATA", data)
        .env_remove("MODN_GAME_MODULES_DIR")
        .args(args)
        .output()
        .expect("CLI")
}

fn success(output: &Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("JSON")
}

#[test]
fn trusted_registry_copies_checks_and_removes_runtime_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    main_bucket::seed(temp.path());
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/game-modules/skyrim-special-edition");
    let source = source.to_str().unwrap();
    let denied = cli(temp.path(), &["module", "register", source]);
    assert!(!denied.status.success());
    assert!(String::from_utf8_lossy(&denied.stderr).contains("trust"));
    assert!(!temp.path().join("modn/game-modules").exists());
    let added = success(&cli(
        temp.path(),
        &["--json", "module", "register", source, "--trust"],
    ));
    assert_eq!(added["module"]["game_id"], "skyrim-special-edition");
    let listed = success(&cli(temp.path(), &["--json", "module", "list"]));
    assert_eq!(listed.as_array().unwrap().len(), 1);
    fs::write(
        temp.path()
            .join("modn/game-modules/skyrim-special-edition/module.lua"),
        "return {}",
    )
    .unwrap();
    assert!(!cli(temp.path(), &["module", "list"]).status.success());
    success(&cli(
        temp.path(),
        &["--json", "module", "remove", "skyrim-special-edition"],
    ));
    assert_eq!(
        success(&cli(temp.path(), &["--json", "module", "list"])),
        serde_json::json!([])
    );
    let official =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../buckets/main/games/cyberpunk-2077");
    assert!(!cli(
        temp.path(),
        &["module", "register", official.to_str().unwrap(), "--trust"]
    )
    .status
    .success());
}

#[test]
fn git_sync_reports_revision_and_preserves_catalog_on_invalid_update() {
    let temp = tempfile::tempdir().unwrap();
    main_bucket::seed(temp.path());
    let repo = temp.path().join("source");
    fs::create_dir(&repo).unwrap();
    fs::write(
        repo.join("bucket.json"),
        r#"{"schema_version":1,"name":"community","title":"Fixture"}"#,
    )
    .unwrap();
    fs::create_dir(repo.join("games")).unwrap();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    };
    git(&["init"]);
    git(&["add", "."]);
    git(&[
        "-c",
        "user.name=Fixture",
        "-c",
        "user.email=fixture@example.invalid",
        "commit",
        "-m",
        "fixture",
    ]);
    let commit = git(&["rev-parse", "HEAD"]);
    let args = [
        "--json",
        "bucket",
        "sync",
        "community",
        "--source",
        repo.to_str().unwrap(),
        "--revision",
        "HEAD",
    ];
    let synced = success(&cli(temp.path(), &args));
    assert_eq!(synced["commit"], commit);
    fs::write(repo.join("module.lua"), "return {}").unwrap();
    git(&["add", "."]);
    git(&[
        "-c",
        "user.name=Fixture",
        "-c",
        "user.email=fixture@example.invalid",
        "commit",
        "-m",
        "invalid",
    ]);
    assert!(!cli(temp.path(), &args).status.success());
    let store = modn_core::buckets::BucketStore::new(&temp.path().join("modn"));
    assert_eq!(
        store.git_revision("community").unwrap().unwrap().commit,
        commit
    );
    assert!(cli(temp.path(), &["bucket", "list"]).status.success());
    assert!(!cli(
        temp.path(),
        &[
            "bucket",
            "sync",
            "main",
            "--source",
            repo.to_str().unwrap(),
            "--revision",
            "HEAD"
        ]
    )
    .status
    .success());
}

#[test]
fn registered_skyrim_runs_cli_lifecycle_without_overwriting_manual_content() {
    use std::io::Write;
    let temp = tempfile::tempdir().unwrap();
    main_bucket::seed(temp.path());
    let example = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/game-modules/skyrim-special-edition");
    let source = temp.path().join("skyrim-module");
    fs::create_dir(&source).unwrap();
    fs::copy(example.join("module.json"), source.join("module.json")).unwrap();
    fs::copy(example.join("module.lua"), source.join("module.lua")).unwrap();
    success(&cli(
        temp.path(),
        &[
            "--json",
            "module",
            "register",
            source.to_str().unwrap(),
            "--trust",
        ],
    ));
    fs::remove_file(source.join("module.lua")).unwrap();
    fs::remove_file(source.join("module.json")).unwrap();
    let game = temp.path().join("game");
    fs::create_dir_all(game.join("Data")).unwrap();
    fs::write(game.join("SkyrimSE.exe"), b"synthetic marker").unwrap();
    fs::write(game.join("Data/manual.esp"), b"manual").unwrap();
    fs::write(game.join("plugins.txt"), b"manual order").unwrap();
    let invalid = temp.path().join("invalid-game");
    fs::create_dir_all(invalid.join("Data")).unwrap();
    let rejected = cli(
        temp.path(),
        &[
            "game",
            "add",
            "skyrim-special-edition",
            "--path",
            invalid.to_str().unwrap(),
        ],
    );
    assert!(!rejected.status.success());
    assert!(!String::from_utf8_lossy(&rejected.stderr).contains("runtime error"));
    let archive = temp.path().join("skyrim.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
    zip.start_file("Data/Example.esp", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"synthetic plugin").unwrap();
    zip.finish().unwrap();
    let game_path = game.to_str().unwrap();
    success(&cli(
        temp.path(),
        &[
            "--json",
            "game",
            "add",
            "skyrim-special-edition",
            "--path",
            game_path,
        ],
    ));
    assert!(
        !cli(temp.path(), &["--module-src", "missing-module", "scan"])
            .status
            .success()
    );
    let installed = success(&cli(
        temp.path(),
        &["--json", "install", archive.to_str().unwrap(), "--yes"],
    ));
    assert_eq!(
        fs::read(game.join("Data/Example.esp")).unwrap(),
        b"synthetic plugin"
    );
    let listed = success(&cli(temp.path(), &["--json", "list"]));
    assert_eq!(listed.as_array().unwrap().len(), 1);
    success(&cli(temp.path(), &["--json", "doctor"]));
    let id = installed["id"].as_str().unwrap();
    success(&cli(temp.path(), &["--json", "uninstall", id, "--yes"]));
    assert!(!game.join("Data/Example.esp").exists());
    assert_eq!(fs::read(game.join("Data/manual.esp")).unwrap(), b"manual");
    assert_eq!(fs::read(game.join("plugins.txt")).unwrap(), b"manual order");
}

#[test]
fn help_exposes_explicit_trust_and_git_revision_controls() {
    let temp = tempfile::tempdir().unwrap();
    for (args, expected) in [
        (
            vec!["module", "--help"],
            vec!["register", "list", "remove", "check"],
        ),
        (vec!["module", "register", "--help"], vec!["--trust"]),
        (
            vec!["bucket", "sync", "--help"],
            vec!["--source", "--revision"],
        ),
    ] {
        let output = cli(temp.path(), &args);
        assert!(output.status.success());
        let help = String::from_utf8(output.stdout).unwrap();
        for text in expected {
            assert!(help.contains(text), "{help}");
        }
    }
    assert!(!temp.path().join("modn").exists());
}
