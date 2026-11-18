//! Read-only catalog, inventory, plan and diagnostics commands.

use super::args;
use super::{installation, run_blocking};
use crate::app::error::CommandError;
use crate::app::state::AppState;
use serde_json::Value;
use std::sync::Arc;
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
    state: tauri::State<'_, AppState>,
) -> Result<(), CommandError> {
    let report = doctor_report(&state).await?;
    write_clipboard(&report, system_clipboard)
}

/// Gives `text` to `write` and maps a refusal to the stable clipboard code.
///
/// Tests pass a fake writer, so they never touch the machine's clipboard.
fn write_clipboard(
    text: &str,
    write: impl FnOnce(&str) -> Result<(), ()>,
) -> Result<(), CommandError> {
    write(text).map_err(|()| CommandError::clipboard_failed())
}

/// Writes one Unicode text block through the Windows clipboard API.
fn system_clipboard(text: &str) -> Result<(), ()> {
    clipboard_win::set_clipboard_string(text).map_err(|_| ())
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

#[cfg(test)]
mod tests {
    use super::write_clipboard;

    #[test]
    fn clipboard_write_passes_the_report_text() {
        let mut copied = String::new();
        let result = write_clipboard("{\"scan\":{}}", |text| {
            copied.push_str(text);
            Ok(())
        });
        assert!(result.is_ok());
        assert_eq!(copied, "{\"scan\":{}}");
    }

    #[test]
    fn clipboard_write_maps_a_refusal_to_the_stable_code() {
        let error = write_clipboard("report", |_| Err(())).expect_err("the writer refused");
        assert_eq!(error.code, "CLIPBOARD_FAILED");
        assert_eq!(
            error.message,
            "Modn could not copy diagnostics to the clipboard."
        );
    }
}
