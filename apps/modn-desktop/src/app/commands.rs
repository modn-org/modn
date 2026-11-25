pub(crate) mod games;
pub(crate) mod history;
pub(crate) mod library;
pub(crate) mod operations;
pub(crate) mod themes;

mod args;
#[cfg(test)]
mod tests;

use crate::app::error::CommandError;
use crate::app::state::{AppState, RunGuard};
use crate::backend::{CancelToken, Game, ProgressEvent};
use serde_json::Value;
use std::ffi::OsString;
use std::path::PathBuf;
use tauri::Emitter;

pub(super) use crate::app::blocking::run_blocking;

#[cfg(test)]
pub(crate) const COMMANDS: &[&str] = &[
    "list_games",
    "list_installations",
    "list_packages",
    "catalog",
    "pick_archive",
    "install_archive",
    "uninstall",
    "install_mod",
    "plan_install",
    "update",
    "history",
    "undo",
    "detect_games",
    "activate_game",
    "add_game",
    "remove_game",
    "play",
    "diagnostics",
    "copy_diagnostics",
    "cancel_run",
    "list_themes",
    "resolve_theme",
    "select_theme",
    "reset_theme",
    "import_theme",
];

const PROGRESS_EVENT: &str = "modn://progress";

pub(super) fn installation(game_id: String, path: String) -> Game {
    Game {
        game_id,
        path: PathBuf::from(path),
    }
}

pub(super) fn ensure_not_cancelled(run: &RunGuard) -> Result<(), CommandError> {
    if run.token().is_cancelled() {
        return Err(CommandError::cancelled());
    }
    Ok(())
}

pub(super) async fn run_write(
    app: &tauri::AppHandle,
    state: &AppState,
    run: RunGuard,
    args: Vec<OsString>,
) -> Result<Value, CommandError> {
    ensure_not_cancelled(&run)?;
    let backend = state.backend()?;
    let cancel: CancelToken = run.token();
    let app = app.clone();
    let event_run_id = run.run_id().to_string();
    let _guard = state.busy.begin();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut on_event = |event: ProgressEvent| {
            emit_progress(&app, &event_run_id, &event);
        };
        backend.call_with_progress(&args, &cancel, &mut on_event)
    })
    .await;
    drop(run);
    match result {
        Ok(Ok(value)) => Ok(with_operation_id(value)),
        Ok(Err(error)) => Err(CommandError::from(error)),
        Err(_) => Err(CommandError::backend_failed()),
    }
}

fn with_operation_id(value: Value) -> Value {
    let mut object = match value {
        Value::Object(object) => object,
        other => return other,
    };
    let operation_id = object
        .get("operation_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if let Some(operation_id) = operation_id {
        object.insert("operationId".to_owned(), Value::String(operation_id));
    }
    Value::Object(object)
}

fn emit_progress(app: &tauri::AppHandle, run_id: &str, event: &ProgressEvent) {
    if let Err(error) = app.emit(PROGRESS_EVENT, progress_payload(run_id, event)) {
        eprintln!("Modn could not deliver a progress event: {error}");
    }
}

fn progress_payload(run_id: &str, event: &ProgressEvent) -> Value {
    serde_json::json!({"runId": run_id, "event": event})
}
