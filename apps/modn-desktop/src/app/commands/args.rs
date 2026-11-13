//! The exact CLI argument vector of every command.
//!
//! Each builder is pure so a unit test can pin the command line. Handlers add
//! no flags of their own: approval flags appear only when the webview asked
//! for them, and `--force` and adoption flags never appear.

use crate::backend::Game;
use std::ffi::OsString;
use std::path::Path;

fn selected(game: &Game) -> Vec<OsString> {
    vec!["--game".into(), game.game_id.clone().into()]
}

fn with_path(mut args: Vec<OsString>, path: &Path) -> Vec<OsString> {
    args.push("--path".into());
    args.push(path.as_os_str().to_os_string());
    args
}

fn with_native_injector(mut args: Vec<OsString>, allowed: bool) -> Vec<OsString> {
    if allowed {
        args.push("--allow-native-injector".into());
    }
    args
}

fn with_executables(mut args: Vec<OsString>, executables: &[String]) -> Vec<OsString> {
    for executable in executables {
        args.push("--allow-executable".into());
        args.push(executable.into());
    }
    args
}

pub(super) fn list_games() -> Vec<OsString> {
    vec!["game".into(), "list".into()]
}

pub(super) fn list_packages(game: &Game) -> Vec<OsString> {
    let mut args = selected(game);
    args.push("list".into());
    with_path(args, &game.path)
}

pub(super) fn catalog(game_id: &str, query: Option<&str>) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["--game".into(), game_id.into(), "search".into()];
    if let Some(query) = query {
        args.push(query.into());
    }
    args
}

pub(super) fn install_archive_dry_run(
    game: &Game,
    archive: &Path,
    allow_native_injector: bool,
) -> Vec<OsString> {
    let mut args = selected(game);
    args.push("install".into());
    args.push(archive.as_os_str().to_os_string());
    args.push("--dry-run".into());
    with_native_injector(with_path(args, &game.path), allow_native_injector)
}

pub(super) fn install_archive_commit(
    game: &Game,
    archive: &Path,
    allow_native_injector: bool,
    executables: &[String],
) -> Vec<OsString> {
    let mut args = selected(game);
    args.push("install".into());
    args.push(archive.as_os_str().to_os_string());
    args.push("--yes".into());
    let args = with_native_injector(with_path(args, &game.path), allow_native_injector);
    with_executables(args, executables)
}

pub(super) fn install_mod_dry_run(
    game: &Game,
    mod_id: &str,
    allow_native_injector: bool,
) -> Vec<OsString> {
    let mut args = selected(game);
    args.push("install".into());
    args.push(mod_id.into());
    args.push("--dry-run".into());
    with_native_injector(with_path(args, &game.path), allow_native_injector)
}

pub(super) fn install_mod_commit(
    game: &Game,
    mod_id: &str,
    allow_native_injector: bool,
    executables: &[String],
) -> Vec<OsString> {
    let mut args = selected(game);
    args.push("install".into());
    args.push(mod_id.into());
    args.push("--yes".into());
    let args = with_native_injector(with_path(args, &game.path), allow_native_injector);
    with_executables(args, executables)
}

pub(super) fn plan_install(game: &Game, mod_id: &str) -> Vec<OsString> {
    let mut args = selected(game);
    args.push("install".into());
    args.push(mod_id.into());
    args.push("--dry-run".into());
    with_path(args, &game.path)
}

pub(super) fn uninstall(game: &Game, package_id: &str) -> Vec<OsString> {
    let mut args = selected(game);
    args.push("uninstall".into());
    args.push(package_id.into());
    args.push("--yes".into());
    with_path(args, &game.path)
}

pub(super) fn update(game: &Game, package_id: &str) -> Vec<OsString> {
    let mut args = selected(game);
    args.push("update".into());
    args.push(package_id.into());
    args.push("--yes".into());
    with_path(args, &game.path)
}

pub(super) fn undo(operation_id: &str) -> Vec<OsString> {
    vec!["undo".into(), operation_id.into(), "--yes".into()]
}

pub(super) fn history(game_id: &str) -> Vec<OsString> {
    vec!["--game".into(), game_id.into(), "history".into()]
}

pub(super) fn detect_games() -> Vec<OsString> {
    vec!["game".into(), "detect".into()]
}

pub(super) fn game_add(game_id: &str, path: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["game".into(), "add".into(), game_id.into()];
    args = with_path(args, path);
    args
}

pub(super) fn game_remove(game_id: &str, path: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["game".into(), "remove".into(), game_id.into()];
    args = with_path(args, path);
    args
}

pub(super) fn play(game_id: &str, path: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["play".into(), game_id.into()];
    args = with_path(args, path);
    args
}

pub(super) fn diagnostics() -> Vec<OsString> {
    vec!["doctor".into()]
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn install_archive_dry_run_and_commit_vectors() {
        assert_eq!(
            strings(install_archive_dry_run(&game(), &archive(), false)),
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
            strings(install_archive_commit(
                &game(),
                &archive(),
                true,
                &["bin/tool.exe".to_string()]
            )),
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
        let commit = strings(install_archive_commit(&game(), &archive(), false, &[]));
        assert!(!commit.iter().any(|arg| arg == "--allow-native-injector"));
        assert!(!commit.iter().any(|arg| arg == "--allow-executable"));
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
    fn install_mod_dry_run_and_commit_vectors() {
        assert_eq!(
            strings(install_mod_dry_run(&game(), "lovely", false)),
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
        assert_eq!(
            strings(install_mod_commit(
                &game(),
                "main/balatro/lovely",
                true,
                &["bin/a.exe".to_string(), "bin/b.exe".to_string()]
            )),
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
    fn update_selects_the_package_and_commits() {
        assert_eq!(
            strings(update(&game(), "txn-1")),
            vec![
                "--game",
                "balatro",
                "update",
                "txn-1",
                "--yes",
                "--path",
                r"C:\games\balatro"
            ]
        );
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
        all.extend(install_archive_dry_run(&game(), &archive(), true));
        all.extend(install_archive_commit(
            &game(),
            &archive(),
            true,
            &["bin/tool.exe".to_string()],
        ));
        all.extend(install_mod_dry_run(&game(), "lovely", true));
        all.extend(install_mod_commit(
            &game(),
            "lovely",
            true,
            &["bin/tool.exe".to_string()],
        ));
        all.extend(uninstall(&game(), "txn-1"));
        all.extend(update(&game(), "txn-1"));
        all.extend(undo("txn-1"));
        let text = strings(all);
        for forbidden in ["--force", "--adopt", "--include-deps", "--cascade"] {
            assert!(
                !text.iter().any(|arg| arg == forbidden),
                "{forbidden} appeared"
            );
        }
    }
}
