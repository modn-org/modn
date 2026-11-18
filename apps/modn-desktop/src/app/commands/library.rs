//! Read-only catalog, inventory, plan and diagnostics commands.

use super::args;
use super::{installation, run_blocking};
use crate::app::error::CommandError;
use crate::app::state::AppState;
use serde_json::Value;
use std::sync::Arc;
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;

#[tauri::command]
pub(crate) async fn list_packages(
    state: tauri::State<'_, AppState>,
    game_id: String,
    path: String,
) -> Result<Value, CommandError> {
    let backend = state.backend()?;
    let busy = Arc::clone(&state.busy);
    let args = args::list_packages(&installation(game_id, path));
    run_blocking(&busy, move || backend.call(&args)).await
}

#[tauri::command]
pub(crate) async fn catalog(
    state: tauri::State<'_, AppState>,
    game_id: String,
    path: String,
    query: Option<String>,
) -> Result<Value, CommandError> {
    let backend = state.backend()?;
    let busy = Arc::clone(&state.busy);
    let _ = path;
    let query = query
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let args = args::catalog(&game_id, query.as_deref());
    run_blocking(&busy, move || backend.call(&args)).await
}

/// Read-only install plan for the details screen. Writes nothing.
#[tauri::command]
pub(crate) async fn plan_install(
    state: tauri::State<'_, AppState>,
    game_id: String,
    path: String,
    mod_id: String,
) -> Result<Value, CommandError> {
    let backend = state.backend()?;
    let busy = Arc::clone(&state.busy);
    let args = args::plan_install(&installation(game_id, path), &mod_id);
    run_blocking(&busy, move || backend.call(&args)).await
}

/// Returns the doctor report as pretty JSON text.
#[tauri::command]
pub(crate) async fn diagnostics(state: tauri::State<'_, AppState>) -> Result<String, CommandError> {
    doctor_report(&state).await
}

/// Copies the doctor report to the Windows clipboard.
///
/// The write runs in the process, not in the webview, so it needs no focused
/// document and keeps working while another window holds the foreground.
#[tauri::command]
pub(crate) async fn copy_diagnostics(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), CommandError> {
    let report = doctor_report(&state).await?;
    app.clipboard()
        .write_text(report)
        .map_err(|_| CommandError::clipboard_failed())
}

/// Builds the doctor report text that support asks for.
async fn doctor_report(state: &AppState) -> Result<String, CommandError> {
    let backend = state.backend()?;
    let busy = Arc::clone(&state.busy);
    let report = run_blocking(&busy, move || backend.call(&args::diagnostics())).await?;
    serde_json::to_string_pretty(&report)
        .map_err(|error| CommandError::from(anyhow::Error::from(error)))
}

/// Shows the native archive picker. The dialog callback fires on the main
/// thread; this command waits on a worker so the webview stays responsive.
#[tauri::command]
pub(crate) async fn pick_archive(app: tauri::AppHandle) -> Result<Option<String>, CommandError> {
    let (sender, receiver) = std::sync::mpsc::channel();
    app.dialog()
        .file()
        .set_title("Choose a local ZIP or 7z archive")
        .add_filter("Mod archives", &["zip", "7z"])
        .pick_file(move |file| {
            let _ = sender.send(file);
        });
    let picked = tauri::async_runtime::spawn_blocking(move || receiver.recv().ok().flatten())
        .await
        .map_err(|_| CommandError::backend_failed())?;
    Ok(picked
        .and_then(|file| file.into_path().ok())
        .map(|path| path.to_string_lossy().into_owned()))
}
