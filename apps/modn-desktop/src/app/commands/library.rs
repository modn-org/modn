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

#[tauri::command]
pub(crate) async fn diagnostics(state: tauri::State<'_, AppState>) -> Result<String, CommandError> {
    doctor_report(&state).await
}

#[tauri::command]
pub(crate) async fn copy_diagnostics(
    state: tauri::State<'_, AppState>,
) -> Result<(), CommandError> {
    let report = doctor_report(&state).await?;
    write_clipboard(&report, destination)
}

fn write_clipboard(
    text: &str,
    write: impl FnOnce(&str) -> Result<(), ()>,
) -> Result<(), CommandError> {
    write(text).map_err(|()| CommandError::clipboard_failed())
}

#[cfg(feature = "verification")]
fn destination(text: &str) -> Result<(), ()> {
    file_sink(std::env::var_os("MODN_VERIFY_CLIPBOARD_FILE"), text)
}

#[cfg(feature = "verification")]
fn file_sink(path: Option<std::ffi::OsString>, text: &str) -> Result<(), ()> {
    let path = path.filter(|value| !value.is_empty()).ok_or(())?;
    std::fs::write(path, text).map_err(|_| ())
}

#[cfg(not(feature = "verification"))]
fn destination(text: &str) -> Result<(), ()> {
    system_clipboard(text)
}

#[cfg(not(feature = "verification"))]
fn system_clipboard(text: &str) -> Result<(), ()> {
    clipboard_win::set_clipboard_string(text).map_err(|_| ())
}

async fn doctor_report(state: &AppState) -> Result<String, CommandError> {
    let backend = state.backend()?;
    let busy = Arc::clone(&state.busy);
    let report = run_blocking(&busy, move || backend.call(&args::diagnostics())).await?;
    serde_json::to_string_pretty(&report)
        .map_err(|error| CommandError::from(anyhow::Error::from(error)))
}

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

    #[cfg(feature = "verification")]
    mod file_sink {
        use super::super::file_sink;
        use std::ffi::OsString;
        use std::path::PathBuf;
        use std::sync::atomic::{AtomicU64, Ordering};

        static SERIAL: AtomicU64 = AtomicU64::new(0);

        fn unique_folder() -> PathBuf {
            let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
            std::env::temp_dir().join(format!(
                "modn-desktop-file-sink-{}-{serial}",
                std::process::id()
            ))
        }

        #[test]
        fn writes_the_exact_text() {
            let folder = unique_folder();
            std::fs::create_dir(&folder).expect("create the unique folder");
            let path = folder.join("clipboard-report.txt");
            let text = "{\n  \"scan\": {}\n}";
            let result = file_sink(Some(path.clone().into_os_string()), text);
            let written = std::fs::read_to_string(&path).expect("read the report");
            std::fs::remove_dir_all(&folder).expect("remove the unique folder");
            assert_eq!(result, Ok(()));
            assert_eq!(written, text);
        }

        #[test]
        fn rejects_a_missing_or_empty_path() {
            assert_eq!(file_sink(None, "report"), Err(()));
            assert_eq!(file_sink(Some(OsString::new()), "report"), Err(()));
        }

        #[test]
        fn reports_a_missing_parent() {
            let folder = unique_folder();
            let path = folder.join("clipboard-report.txt");
            assert_eq!(file_sink(Some(path.into_os_string()), "report"), Err(()));
            assert!(!folder.exists());
        }
    }
}
