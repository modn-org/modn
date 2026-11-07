#[path = "support/main_bucket.rs"]
mod main_bucket;
use std::fs;
use std::process::{Command, Output};

fn run(data: &std::path::Path, args: &[&str]) -> Output {
    Command::new(main_bucket::modn_binary())
        .env("LOCALAPPDATA", data)
        .env_remove("MODN_GAME_MODULES_DIR")
        .args(args)
        .output()
        .expect("run modn")
}

#[test]
fn missing_main_copy_names_expected_path_without_panicking() {
    let data = tempfile::tempdir().expect("fixture invariant");
    let game = tempfile::tempdir().expect("fixture game");
    for args in [
        vec!["search"],
        vec![
            "--game",
            "cyberpunk-2077",
            "scan",
            "--path",
            game.path().to_str().expect("fixture path"),
        ],
    ] {
        let result = run(data.path(), &args);
        assert!(!result.status.success());
        let error = String::from_utf8_lossy(&result.stderr);
        assert!(error.contains("cannot load local main bucket"), "{error}");
        assert!(error.contains("modn bucket update"), "{error}");
        assert!(
            error.replace('\\', "/").contains(
                &data
                    .path()
                    .join("modn/buckets/main")
                    .display()
                    .to_string()
                    .replace('\\', "/")
            ),
            "{error}"
        );
        assert!(!error.contains("panicked"), "{error}");
    }
    assert!(run(data.path(), &["--help"]).status.success());
    assert!(fs::read_dir(data.path())
        .expect("fixture invariant")
        .next()
        .is_none());
}

#[test]
fn official_cyberpunk_module_loads_from_local_main_copy() {
    let data = tempfile::tempdir().expect("fixture invariant");
    main_bucket::seed(data.path());
    let script = data
        .path()
        .join("modn/buckets/main/games/cyberpunk-2077/module.lua");
    let original = fs::read_to_string(&script).expect("fixture invariant");
    fs::write(
        &script,
        original.replace("bin/x64/Cyberpunk2077.exe", "bin/x64/LocalMain.exe"),
    )
    .expect("fixture invariant");
    let game = data.path().join("game");
    fs::create_dir_all(game.join("bin/x64")).expect("fixture invariant");
    fs::write(game.join("bin/x64/LocalMain.exe"), b"fixture").expect("fixture invariant");
    let result = run(
        data.path(),
        &[
            "game",
            "add",
            "cyberpunk-2077",
            "--path",
            game.to_str().expect("fixture invariant"),
        ],
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        fs::read(game.join("bin/x64/LocalMain.exe")).expect("fixture invariant"),
        b"fixture"
    );
}

#[test]
fn registered_local_bucket_cannot_supply_game_module_files() {
    let data = tempfile::tempdir().expect("fixture invariant");
    main_bucket::seed(data.path());
    let bucket = data.path().join("community");
    fs::create_dir_all(bucket.join("games/test-game")).expect("fixture invariant");
    fs::write(
        bucket.join("bucket.json"),
        r#"{"schema_version":1,"name":"community","title":"Community"}"#,
    )
    .expect("fixture invariant");
    for name in ["module.json", "module.lua"] {
        fs::write(bucket.join("games/test-game").join(name), b"invalid")
            .expect("fixture invariant");
        let result = run(
            data.path(),
            &[
                "bucket",
                "add",
                "community",
                bucket.to_str().expect("fixture invariant"),
            ],
        );
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr)
            .contains("only the official main bucket may contain game modules"));
        assert!(!data.path().join("modn/buckets/registrations").exists());
        fs::remove_file(bucket.join("games/test-game").join(name)).expect("fixture invariant");
    }
}
