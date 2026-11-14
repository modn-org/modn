//! Argument-vector tests for the fixed CLI contract.

use super::*;
use std::path::PathBuf;

fn strings(args: Vec<OsString>) -> Vec<String> {
    args.into_iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

fn game() -> Game {
    Game {
        game_id: "balatro".into(),
        path: PathBuf::from(r"C:\games\balatro"),
    }
}

fn archive() -> PathBuf {
    PathBuf::from(r"C:\mods\example.zip")
}

/// One `modn list --json` inventory: a catalog package and a standalone one.
fn installed() -> serde_json::Value {
    serde_json::json!([
        {
            "id": "txn-1",
            "catalog": {"bucket": "main", "game_id": "balatro", "mod_id": "lovely"},
        },
        {
            "id": "txn-2",
            "catalog": null,
            "mod_id": "fixture-mod",
        }
    ])
}

#[test]
fn list_games_runs_the_registration_listing() {
    assert_eq!(strings(list_games()), vec!["game", "list"]);
}

#[test]
fn list_packages_selects_the_installation() {
    assert_eq!(
        strings(list_packages(&game())),
        vec!["--game", "balatro", "list", "--path", r"C:\games\balatro"]
    );
}

#[test]
fn catalog_passes_one_trimmed_query() {
    assert_eq!(
        strings(catalog("balatro", None)),
        vec!["--game", "balatro", "search"]
    );
    assert_eq!(
        strings(catalog("balatro", Some("lovely"))),
        vec!["--game", "balatro", "search", "lovely"]
    );
}

fn ok(args: Result<Vec<OsString>, CommandError>) -> Vec<OsString> {
    args.expect("valid argument vector")
}

#[test]
fn install_archive_dry_run_and_commit_vectors() {
    assert_eq!(
        strings(ok(install_archive_dry_run(&game(), &archive(), false, &[]))),
        vec![
            "--game",
            "balatro",
            "install",
            r"C:\mods\example.zip",
            "--dry-run",
            "--path",
            r"C:\games\balatro"
        ]
    );
    assert_eq!(
        strings(ok(install_archive_commit(
            &game(),
            &archive(),
            true,
            &["bin/tool.exe".to_string()]
        ))),
        vec![
            "--game",
            "balatro",
            "install",
            r"C:\mods\example.zip",
            "--yes",
            "--path",
            r"C:\games\balatro",
            "--allow-native-injector",
            "--allow-executable",
            "bin/tool.exe"
        ]
    );
}

#[test]
fn install_archive_passes_no_approval_until_the_webview_asks() {
    let commit = strings(ok(install_archive_commit(&game(), &archive(), false, &[])));
    assert!(!commit.iter().any(|arg| arg == "--allow-native-injector"));
    assert!(!commit.iter().any(|arg| arg == "--allow-executable"));
}

#[test]
fn commit_emits_one_flag_pair_for_zero_one_and_two_executables() {
    let zero = strings(ok(install_mod_commit(&game(), "lovely", false, &[])));
    assert!(!zero.iter().any(|arg| arg == "--allow-executable"));

    let one = strings(ok(install_mod_commit(
        &game(),
        "lovely",
        false,
        &["bin/a.exe".to_string()],
    )));
    assert_eq!(
        one.iter()
            .filter(|arg| arg.as_str() == "--allow-executable")
            .count(),
        1
    );
    assert!(
        one.windows(2)
            .any(|pair| pair == ["--allow-executable", "bin/a.exe"])
    );

    let two = strings(ok(install_archive_commit(
        &game(),
        &archive(),
        false,
        &["bin/a.exe".to_string(), "bin/b.exe".to_string()],
    )));
    assert_eq!(
        two.iter()
            .filter(|arg| arg.as_str() == "--allow-executable")
            .count(),
        2
    );
    assert!(
        two.windows(2)
            .any(|pair| pair == ["--allow-executable", "bin/a.exe"])
    );
    assert!(
        two.windows(2)
            .any(|pair| pair == ["--allow-executable", "bin/b.exe"])
    );
}

#[test]
fn unsafe_executable_paths_are_rejected_without_a_process() {
    for path in [
        "",
        r"C:\tools\tool.exe",
        "C:tool.exe",
        "/tools/tool.exe",
        r"\tools\tool.exe",
        "bin/../tool.exe",
        r"bin\..\tool.exe",
        "--allow-native-injector",
        "-x",
    ] {
        let error = install_archive_commit(&game(), &archive(), false, &[path.to_string()])
            .expect_err("unsafe executable path must be rejected");
        assert_eq!(error.code, "INVALID_EXECUTABLE", "path: {path:?}");
        assert!(!error.message.is_empty(), "path: {path:?}");
    }
}

#[test]
fn uninstall_selects_the_package_and_never_forces() {
    assert_eq!(
        strings(uninstall(&game(), "txn-1")),
        vec![
            "--game",
            "balatro",
            "uninstall",
            "txn-1",
            "--yes",
            "--path",
            r"C:\games\balatro"
        ]
    );
}

#[test]
fn install_mod_commit_vector() {
    assert_eq!(
        strings(ok(install_mod_commit(
            &game(),
            "main/balatro/lovely",
            true,
            &["bin/a.exe".to_string(), "bin/b.exe".to_string()]
        ))),
        vec![
            "--game",
            "balatro",
            "install",
            "main/balatro/lovely",
            "--yes",
            "--path",
            r"C:\games\balatro",
            "--allow-native-injector",
            "--allow-executable",
            "bin/a.exe",
            "--allow-executable",
            "bin/b.exe"
        ]
    );
}

#[test]
fn plan_install_is_a_dry_run_without_approvals() {
    assert_eq!(
        strings(plan_install(&game(), "lovely")),
        vec![
            "--game",
            "balatro",
            "install",
            "lovely",
            "--dry-run",
            "--path",
            r"C:\games\balatro"
        ]
    );
}

#[test]
fn update_selects_the_qualified_catalog_name_and_commits() {
    assert_eq!(
        strings(ok(update(&game(), &installed(), "txn-1", false, &[]))),
        vec![
            "--game",
            "balatro",
            "update",
            "main/balatro/lovely",
            "--yes",
            "--path",
            r"C:\games\balatro"
        ]
    );
    assert_eq!(
        strings(ok(update(
            &game(),
            &installed(),
            "txn-1",
            true,
            &["bin/tool.exe".to_string()]
        ))),
        vec![
            "--game",
            "balatro",
            "update",
            "main/balatro/lovely",
            "--yes",
            "--path",
            r"C:\games\balatro",
            "--allow-native-injector",
            "--allow-executable",
            "bin/tool.exe"
        ]
    );
}

#[test]
fn update_refuses_a_package_without_a_catalog_name() {
    let error = update(&game(), &installed(), "txn-2", false, &[])
        .expect_err("a standalone package has no update target");
    assert_eq!(error.code, "UPDATE_FAILED");
}

#[test]
fn update_refuses_an_unknown_package() {
    let error = update(&game(), &installed(), "txn-missing", false, &[])
        .expect_err("the inventory does not list this package");
    assert_eq!(error.code, "MOD_NOT_FOUND");
}

#[test]
fn history_uses_the_game_selection() {
    assert_eq!(
        strings(history("balatro")),
        vec!["--game", "balatro", "history"]
    );
}

#[test]
fn undo_names_the_operation_and_confirms() {
    assert_eq!(
        strings(undo("txn-SmOXgO")),
        vec!["undo", "txn-SmOXgO", "--yes"]
    );
}

#[test]
fn detect_games_never_registers() {
    let args = strings(detect_games());
    assert_eq!(args, vec!["game", "detect"]);
    assert!(!args.iter().any(|arg| arg == "--register"));
}

#[test]
fn game_add_registers_the_chosen_folder() {
    assert_eq!(
        strings(game_add("balatro", Path::new(r"C:\games\balatro"))),
        vec!["game", "add", "balatro", "--path", r"C:\games\balatro"]
    );
}

#[test]
fn game_remove_names_the_installation() {
    assert_eq!(
        strings(game_remove("balatro", Path::new(r"C:\games\balatro"))),
        vec!["game", "remove", "balatro", "--path", r"C:\games\balatro"]
    );
}

#[test]
fn play_selects_the_installation() {
    assert_eq!(
        strings(play("balatro", Path::new(r"C:\games\balatro"))),
        vec!["play", "balatro", "--path", r"C:\games\balatro"]
    );
}

#[test]
fn diagnostics_runs_doctor() {
    assert_eq!(strings(diagnostics()), vec!["doctor"]);
}

#[test]
fn no_write_builder_emits_force_or_adoption_flags() {
    let mut all = Vec::new();
    all.extend(ok(install_archive_dry_run(
        &game(),
        &archive(),
        true,
        &["bin/tool.exe".to_string()],
    )));
    all.extend(ok(install_archive_commit(
        &game(),
        &archive(),
        true,
        &["bin/tool.exe".to_string()],
    )));
    all.extend(ok(install_mod_commit(
        &game(),
        "lovely",
        true,
        &["bin/tool.exe".to_string()],
    )));
    all.extend(uninstall(&game(), "txn-1"));
    all.extend(ok(update(&game(), &installed(), "txn-1", true, &[])));
    all.extend(undo("txn-1"));
    let text = strings(all);
    for forbidden in ["--force", "--adopt", "--include-deps", "--cascade"] {
        assert!(
            !text.iter().any(|arg| arg == forbidden),
            "{forbidden} appeared"
        );
    }
}
