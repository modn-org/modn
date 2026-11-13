//! Operation history and undo.

use super::args;
use super::run_write;
use crate::app::error::CommandError;
use crate::app::state::AppState;
use serde_json::Value;
use std::sync::Arc;

#[tauri::command]
pub(crate) async fn history(
    state: tauri::State<'_, AppState>,
    game_id: String,
) -> Result<Value, CommandError> {
    let backend = state.backend()?;
    let busy = Arc::clone(&state.busy);
    let args = args::history(&game_id);
    super::run_blocking(&busy, move || backend.call(&args)).await
}

#[tauri::command]
pub(crate) async fn undo(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    operation_id: String,
    run_id: String,
) -> Result<Value, CommandError> {
    let run = state.runs.start(&run_id);
    let commit = args::undo(&operation_id);
    run_write(&app, &state, run, commit).await
}
