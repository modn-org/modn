//! The webview's only entry points into the backend.
//!
//! Every command validates the webview's selection, builds one fixed argument
//! vector with [`args`] and runs the configured CLI off the UI thread. Write
//! commands stream CLI progress events to the main window and hold a cancel
//! token until they finish.

pub(crate) mod games;
pub(crate) mod history;
pub(crate) mod library;
pub(crate) mod operations;

mod args;
#[cfg(test)]
mod tests;

use crate::app::error::CommandError;
use crate::app::state::{AppState, Busy};
use crate::backend::{CancelToken, Game, ProgressEvent};
use serde_json::Value;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Emitter;

/// The name of every command registered in `generate_handler!`.
///
/// The capability file, the build manifest and `ui/js/api.js` must list the
/// same names; the unit tests in this module check all four lists.
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
    "cancel_run",
];

/// The progress event name the webview subscribes to.
const PROGRESS_EVENT: &str = "modn://progress";

pub(super) fn installation(game_id: String, path: String) -> Game {
    Game {
        game_id,
        path: PathBuf::from(path),
    }
}

/// Runs one backend call off the UI thread while holding the busy guard that
/// vetoes a window close.
pub(super) async fn run_blocking<T>(
    busy: &Arc<Busy>,
    job: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> Result<T, CommandError>
where
    T: Send + 'static,
{
    let _guard = busy.begin();
    match tauri::async_runtime::spawn_blocking(job).await {
        Ok(result) => result.map_err(CommandError::from),
        Err(_) => Err(CommandError::backend_failed()),
    }
}

/// Runs one write command with `--progress json`, forwards every event to the
/// main window and removes the cancel token when the run ends.
pub(super) async fn run_write(
    app: &tauri::AppHandle,
    state: &AppState,
    run_id: String,
    args: Vec<OsString>,
) -> Result<Value, CommandError> {
    let backend = state.backend()?;
    let run = state.runs.start(&run_id);
    let cancel: CancelToken = run.token();
    let app = app.clone();
    let event_run_id = run_id.clone();
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
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(CommandError::from(error)),
        Err(_) => Err(CommandError::backend_failed()),
    }
}

/// Sends one CLI progress event to the webview unchanged.
fn emit_progress(app: &tauri::AppHandle, run_id: &str, event: &ProgressEvent) {
    if let Err(error) = app.emit(PROGRESS_EVENT, progress_payload(run_id, event)) {
        eprintln!("Modn could not deliver a progress event: {error}");
    }
}

fn progress_payload(run_id: &str, event: &ProgressEvent) -> Value {
    serde_json::json!({"runId": run_id, "event": event})
}
