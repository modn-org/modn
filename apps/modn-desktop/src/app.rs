mod commands;
mod error;
mod state;

use crate::backend::Backend;
use state::AppState;
use tauri::{Manager, WindowEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

/// Runs the Tauri application. Startup configuration errors are reported in a
/// native dialog because the Windows subsystem has no console.
pub(crate) fn run() {
    let (backend, startup_error) = match Backend::from_args() {
        Ok(backend) => (Some(backend), None),
        Err(error) => (None, Some(format!("{error:#}"))),
    };
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new(backend))
        .invoke_handler(tauri::generate_handler![
            commands::list_games,
            commands::list_installations,
            commands::list_packages,
            commands::catalog,
            commands::pick_archive,
            commands::install_archive,
            commands::uninstall,
        ])
        .setup(move |app| {
            if let Some(message) = startup_error.as_deref() {
                app.dialog()
                    .message(format!("Cannot start Modn Desktop: {message}"))
                    .title("Modn")
                    .kind(MessageDialogKind::Error)
                    .blocking_show();
                app.handle().exit(1);
                return Ok(());
            }
            guard_close(app);
            Ok(())
        })
        .run(tauri::generate_context!());
    if let Err(error) = result {
        eprintln!("Modn Desktop failed: {error}");
        std::process::exit(1);
    }
}

fn guard_close(app: &mut tauri::App) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let handle = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            if handle.state::<AppState>().busy.is_busy() {
                api.prevent_close();
                handle
                    .dialog()
                    .message("Modn is still working. Wait for the operation to finish.")
                    .title("Modn")
                    .kind(MessageDialogKind::Warning)
                    .show(|_| {});
            }
        }
    });
}
