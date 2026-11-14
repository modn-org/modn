use super::{plan_launch, LaunchTarget};
use crate::detection::DetectedInstallation;
use crate::game_modules::with_module_source;
use crate::state::Store;
use crate::stores::Storefront;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn module(launch: &str, stores: &str) -> TempDir {
    let temp = tempfile::tempdir().expect("module fixture");
    fs::write(
        temp.path().join("module.json"),
        format!(
            r#"{{"game_id":"launch-game","version":"0.1.0","api_version":1,"entrypoint":"main.lua","stores":{stores},"launch":{launch}}}"#
        ),
    )
    .expect("manifest");
    fs::write(
        temp.path().join("main.lua"),
        r#"return { game_id = "launch-game", api_version = mods_host.api_version }"#,
    )
    .expect("script");
    temp
}

fn game() -> TempDir {
    let temp = tempfile::tempdir().expect("game fixture");
    fs::write(temp.path().join("Game.exe"), b"fixture").expect("game executable");
    temp
}

fn canonical(temp: &TempDir) -> PathBuf {
    temp.path().canonicalize().expect("canonical game")
}

fn steam_detection(path: &Path) -> Vec<DetectedInstallation> {
    vec![DetectedInstallation {
        game_id: "launch-game".into(),
        store: Storefront::Steam,
        store_id: "1091500".into(),
        path: path.to_path_buf(),
    }]
}

#[test]
fn prefers_the_store_uri_when_detection_matches_the_folder() {
    let module = module(
        r#"{"steam":"steam://rungameid/1091500","executable":"Game.exe"}"#,
        r#"{"steam":"1091500"}"#,
    );
    let game = game();
    let installation = canonical(&game);
    let detected = steam_detection(&installation);

    let plan = with_module_source(Some(module.path()), || {
        plan_launch(None, "launch-game", &installation, &detected)
    })
    .expect("launch plan");

    assert_eq!(
        plan.target,
        LaunchTarget::StoreUri {
            store: Storefront::Steam,
            uri: "steam://rungameid/1091500".into(),
        }
    );
    let report = serde_json::to_value(&plan).expect("JSON plan");
    assert_eq!(report["target"]["kind"], "store_uri");
    assert_eq!(report["target"]["store"], "steam");
    assert_eq!(report["target"]["uri"], "steam://rungameid/1091500");
}

#[test]
fn falls_back_to_the_executable_without_store_detection() {
    let module = module(
        r#"{"steam":"steam://rungameid/1091500","executable":"Game.exe"}"#,
        r#"{"steam":"1091500"}"#,
    );
    let game = game();
    let installation = canonical(&game);

    let plan = with_module_source(Some(module.path()), || {
        plan_launch(None, "launch-game", &installation, &[])
    })
    .expect("launch plan");

    assert_eq!(
        plan.target,
        LaunchTarget::Executable {
            path: installation.join("Game.exe"),
            working_directory: installation.clone(),
        }
    );
}

#[test]
fn refuses_when_no_declared_target_applies() {
    let module = module(
        r#"{"steam":"steam://rungameid/1091500"}"#,
        r#"{"steam":"1091500"}"#,
    );
    let game = game();
    let installation = canonical(&game);

    let error = with_module_source(Some(module.path()), || {
        plan_launch(None, "launch-game", &installation, &[])
    })
    .expect_err("no executable fallback");

    assert!(
        format!("{error:#}").starts_with("LAUNCH_TARGET_UNAVAILABLE:"),
        "{error:#}"
    );
}

#[test]
fn refuses_an_executable_that_resolves_outside_the_game() {
    let module = module(r#"{"executable":"escape.exe"}"#, "{}");
    let game = game();
    let outside = tempfile::NamedTempFile::new().expect("outside executable");
    let link = game.path().join("escape.exe");
    if symlink_file(outside.path(), &link).is_err() {
        eprintln!("skipping: this session cannot create file symlinks");
        return;
    }
    let installation = canonical(&game);

    let error = with_module_source(Some(module.path()), || {
        plan_launch(None, "launch-game", &installation, &[])
    })
    .expect_err("escaping executable");

    let message = format!("{error:#}");
    assert!(message.starts_with("LAUNCH_TARGET_UNSAFE:"), "{message}");
    assert!(message.contains("outside"), "{message}");
}

#[test]
fn refuses_a_missing_executable() {
    let module = module(r#"{"executable":"missing.exe"}"#, "{}");
    let game = game();
    let installation = canonical(&game);

    let error = with_module_source(Some(module.path()), || {
        plan_launch(None, "launch-game", &installation, &[])
    })
    .expect_err("missing executable");

    assert!(
        format!("{error:#}").starts_with("LAUNCH_TARGET_UNSAFE:"),
        "{error:#}"
    );
}

#[test]
fn refuses_to_launch_while_recovery_is_pending() {
    let module = module(r#"{"executable":"Game.exe"}"#, "{}");
    let game = game();
    let installation = canonical(&game);
    let state = tempfile::tempdir().expect("state");
    let store = Store::open(&state.path().join("state.sqlite")).expect("state store");
    store
        .connection
        .execute(
            "INSERT INTO pending_transactions
             (id, action, status, game_path, package_id, archive_blake3)
             VALUES ('pending', 'install', 'pending', ?1, 'fixture', NULL)",
            [installation.to_string_lossy().as_ref()],
        )
        .expect("pending transaction");

    let error = with_module_source(Some(module.path()), || {
        plan_launch(Some(&store), "launch-game", &installation, &[])
    })
    .expect_err("pending recovery");

    assert!(
        format!("{error:#}").starts_with("LAUNCH_RECOVERY_PENDING:"),
        "{error:#}"
    );
}

#[test]
fn warns_about_changed_managed_files_without_refusing() {
    let module = module(r#"{"executable":"Game.exe"}"#, "{}");
    let game = game();
    let installation = canonical(&game);
    let state = tempfile::tempdir().expect("state");
    let store = Store::open(&state.path().join("state.sqlite")).expect("state store");
    let game_text = installation.to_string_lossy().into_owned();
    store
        .connection
        .execute(
            "INSERT INTO managed_packages (id, game_path, archive_blake3)
             VALUES ('fixture', ?1, 'fixture')",
            [&game_text],
        )
        .expect("managed package");
    store
        .connection
        .execute(
            "INSERT INTO managed_files
             (game_path, path_key, relative_path, package_id, blake3, bytes)
             VALUES (?1, 'game.exe', 'Game.exe', 'fixture', 'fixture', 8)",
            [&game_text],
        )
        .expect("managed file");
    fs::write(installation.join("Game.exe"), b"changed").expect("changed file");

    let plan = with_module_source(Some(module.path()), || {
        plan_launch(Some(&store), "launch-game", &installation, &[])
    })
    .expect("launch must continue with a warning");

    assert_eq!(plan.warnings.len(), 1, "{plan:#?}");
    assert!(
        plan.warnings[0].starts_with("LAUNCH_MANAGED_FILES_CHANGED:"),
        "{}",
        plan.warnings[0]
    );
    assert!(matches!(plan.target, LaunchTarget::Executable { .. }));
}

#[cfg(windows)]
fn symlink_file(original: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(original, link)
}

#[cfg(unix)]
fn symlink_file(original: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(original, link)
}
