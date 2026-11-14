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
use crate::app::state::{AppState, RunGuard};
use crate::backend::{CancelToken, Game, ProgressEvent};
use serde_json::Value;
use std::ffi::OsString;
use std::path::PathBuf;
use tauri::Emitter;

pub(super) use crate::app::blocking::run_blocking;

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

/// Refuses a write whose cancel arrived while its checks ran.
pub(super) fn ensure_not_cancelled(run: &RunGuard) -> Result<(), CommandError> {
    if run.token().is_cancelled() {
        return Err(CommandError::cancelled());
    }
    Ok(())
}

/// Runs one write command with `--progress json`, forwards every event to the
/// main window and removes the cancel token when the run ends.
///
/// The caller registers the run before any pre-write check, so a cancel that
/// arrives during those checks reaches this token and stops the write here.
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
