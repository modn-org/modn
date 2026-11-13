//! Write commands: every one streams progress and can be cancelled before the
//! CLI's commit point.

use super::args;
use super::{installation, run_blocking, run_write};
use crate::app::error::{CommandError, plan_blocked};
use crate::app::state::AppState;
use crate::backend::reviewable;
use serde_json::Value;
use std::path::PathBuf;

#[tauri::command]
#[expect(
    clippy::too_many_arguments,
    reason = "The webview contract names every approval and run field explicitly"
)]
pub(crate) async fn install_archive(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    game_id: String,
    path: String,
    archive: String,
    allow_native_injector: bool,
    allow_executables: Vec<String>,
    run_id: String,
) -> Result<Value, CommandError> {
    let backend = state.backend()?;
    let game = installation(game_id, path);
    let archive = PathBuf::from(archive);
    let (plan, archive) = if allow_native_injector || !allow_executables.is_empty() {
        let dry_run = args::install_archive_dry_run(
            &game,
            &archive,
            allow_native_injector,
            &allow_executables,
        )?;
        let backend = backend.clone();
        let plan = run_blocking(&state.busy, move || backend.call(&dry_run)).await?;
        (plan, archive)
    } else {
        let backend = backend.clone();
        let reviewed_game = game.clone();
        let reviewed_archive = archive.clone();
        let review = run_blocking(&state.busy, move || {
            backend.review(&reviewed_game, &reviewed_archive)
        })
        .await?;
        (review.plan, review.archive)
    };
    if !reviewable(&plan) {
        return Err(plan_blocked().into());
    }
    let commit =
        args::install_archive_commit(&game, &archive, allow_native_injector, &allow_executables)?;
    run_write(&app, &state, run_id, commit).await
}

#[tauri::command]
#[expect(
    clippy::too_many_arguments,
    reason = "The webview contract names every approval and run field explicitly"
)]
pub(crate) async fn install_mod(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    game_id: String,
    path: String,
    mod_id: String,
    allow_native_injector: bool,
    allow_executables: Vec<String>,
    run_id: String,
) -> Result<Value, CommandError> {
    let game = installation(game_id, path);
    let commit =
        args::install_mod_commit(&game, &mod_id, allow_native_injector, &allow_executables)?;
    run_write(&app, &state, run_id, commit).await
}

#[tauri::command]
pub(crate) async fn uninstall(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    game_id: String,
    path: String,
    package_id: String,
    run_id: String,
) -> Result<Value, CommandError> {
    let commit = args::uninstall(&installation(game_id, path), &package_id);
    run_write(&app, &state, run_id, commit).await
}

#[tauri::command]
#[expect(
    clippy::too_many_arguments,
    reason = "The webview contract names every approval and run field explicitly"
)]
pub(crate) async fn update(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    game_id: String,
    path: String,
    package_id: String,
    allow_native_injector: bool,
    allow_executables: Vec<String>,
    run_id: String,
) -> Result<Value, CommandError> {
    let commit = args::update(
        &installation(game_id, path),
        &package_id,
        allow_native_injector,
        &allow_executables,
    )?;
    run_write(&app, &state, run_id, commit).await
}

/// Cancels a running write. An unknown run id is a no-op.
#[tauri::command]
pub(crate) fn cancel_run(
    state: tauri::State<'_, AppState>,
    run_id: String,
) -> Result<(), CommandError> {
    state.runs.cancel(&run_id);
    Ok(())
}
