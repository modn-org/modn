//! The exact CLI argument vector of every command.
//!
//! Each builder is pure so a unit test can pin the command line. Handlers add
//! no flags of their own: approval flags appear only when the webview asked
//! for them, and `--force` and adoption flags never appear.

use crate::app::error::CommandError;
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

fn with_executables(
    mut args: Vec<OsString>,
    executables: &[String],
) -> Result<Vec<OsString>, CommandError> {
    for executable in executables {
        if !game_relative(executable) {
            return Err(CommandError::invalid_executable());
        }
        args.push("--allow-executable".into());
        args.push(executable.into());
    }
    Ok(args)
}

/// Whether `executable` is a non-empty game-relative path that cannot be
/// mistaken for a flag or escape the game folder.
///
/// Checks separators on every host so the same paths are rejected on CI and on
/// Windows, where the desktop runs.
fn game_relative(executable: &str) -> bool {
    let bytes = executable.as_bytes();
    let drive_prefix = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    !executable.is_empty()
        && !executable.starts_with('-')
        && !executable.starts_with('/')
        && !executable.starts_with('\\')
        && !drive_prefix
        && executable
            .split(['/', '\\'])
            .all(|component| component != "..")
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
    executables: &[String],
) -> Result<Vec<OsString>, CommandError> {
    let mut args = selected(game);
    args.push("install".into());
    args.push(archive.as_os_str().to_os_string());
    args.push("--dry-run".into());
    let args = with_native_injector(with_path(args, &game.path), allow_native_injector);
    with_executables(args, executables)
}

pub(super) fn install_archive_commit(
    game: &Game,
    archive: &Path,
    allow_native_injector: bool,
    executables: &[String],
) -> Result<Vec<OsString>, CommandError> {
    let mut args = selected(game);
    args.push("install".into());
    args.push(archive.as_os_str().to_os_string());
    args.push("--yes".into());
    let args = with_native_injector(with_path(args, &game.path), allow_native_injector);
    with_executables(args, executables)
}

pub(super) fn install_mod_commit(
    game: &Game,
    mod_id: &str,
    allow_native_injector: bool,
    executables: &[String],
) -> Result<Vec<OsString>, CommandError> {
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

pub(super) fn update(
    game: &Game,
    package_id: &str,
    allow_native_injector: bool,
    executables: &[String],
) -> Result<Vec<OsString>, CommandError> {
    let mut args = selected(game);
    args.push("update".into());
    args.push(package_id.into());
    args.push("--yes".into());
    let args = with_native_injector(with_path(args, &game.path), allow_native_injector);
    with_executables(args, executables)
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
mod tests;
