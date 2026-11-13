use crate::app::error::{CommandError, plan_blocked};
use crate::app::state::{AppState, Busy};
use crate::backend::{Backend, Game, reviewable};
use serde_json::Value;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri_plugin_dialog::DialogExt;

/// Runs one backend operation off the UI thread while holding the busy guard
/// that vetoes a window close.
async fn run_blocking<T>(
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

fn installation(game_id: String, path: String) -> Game {
    Game {
        game_id,
        path: PathBuf::from(path),
    }
}

#[tauri::command]
pub(crate) async fn list_games(state: tauri::State<'_, AppState>) -> Result<Value, CommandError> {
    let backend = state.backend()?;
    let busy = Arc::clone(&state.busy);
    run_blocking(&busy, move || Ok(serde_json::to_value(backend.games()?)?)).await
}

#[tauri::command]
pub(crate) async fn list_installations(
    state: tauri::State<'_, AppState>,
    game_id: String,
) -> Result<Value, CommandError> {
    let backend = state.backend()?;
    let busy = Arc::clone(&state.busy);
    run_blocking(&busy, move || {
        let games: Vec<Game> = backend
            .games()?
            .into_iter()
            .filter(|game| game.game_id == game_id)
            .collect();
        Ok(serde_json::to_value(games)?)
    })
    .await
}

#[tauri::command]
pub(crate) async fn list_packages(
    state: tauri::State<'_, AppState>,
    game_id: String,
    path: String,
) -> Result<Value, CommandError> {
    let backend = state.backend()?;
    let busy = Arc::clone(&state.busy);
    let game = installation(game_id, path);
    run_blocking(&busy, move || backend.read(&game, "list")).await
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
    let mut args: Vec<OsString> = vec!["--game".into(), game_id.into(), "search".into()];
    if let Some(query) = query.map(|value| value.trim().to_string())
        && !query.is_empty()
    {
        args.push(query.into());
    }
    run_blocking(&busy, move || backend.call(&args)).await
}

#[tauri::command]
pub(crate) async fn install_archive(
    state: tauri::State<'_, AppState>,
    game_id: String,
    path: String,
    archive: String,
    allow_native_injector: bool,
    allow_executable: bool,
    run_id: String,
) -> Result<Value, CommandError> {
    let backend = state.backend()?;
    let busy = Arc::clone(&state.busy);
    let game = installation(game_id, path);
    let archive = PathBuf::from(archive);
    let _ = run_id;
    run_blocking(&busy, move || {
        install_one_click(
            &backend,
            &game,
            &archive,
            allow_native_injector,
            allow_executable,
        )
    })
    .await
}

/// One click: run the dry run internally, refuse a blocked plan, recheck the
/// archive and plan, then write. No review or confirmation screen exists.
fn install_one_click(
    backend: &Backend,
    game: &Game,
    archive: &Path,
    allow_native_injector: bool,
    allow_executable: bool,
) -> anyhow::Result<Value> {
    if !allow_native_injector && !allow_executable {
        let review = backend.review(game, archive)?;
        // The review keeps the originally chosen path for evidence; the
        // installation itself uses the canonical archive path.
        let _chosen_source = review.source.as_path();
        if !reviewable(&review.plan) {
            return Err(plan_blocked());
        }
        return backend.install(&review);
    }
    let plan = backend.call(&archive_args(
        game,
        archive,
        &["--dry-run"],
        allow_native_injector,
    ))?;
    if !reviewable(&plan) {
        return Err(plan_blocked());
    }
    let mut args = archive_args(game, archive, &["--yes"], allow_native_injector);
    if allow_executable {
        for executable in declared_executables(&plan) {
            args.push("--allow-executable".into());
            args.push(executable.into());
        }
    }
    backend.call(&args)
}

/// Reads executable paths that the dry run reports as uncovered. The UI never
/// supplies these paths; they come from the CLI plan.
fn declared_executables(plan: &Value) -> Vec<String> {
    let mut paths = Vec::new();
    let uncovered = plan
        .get("approval")
        .and_then(|approval| approval.get("uncovered"))
        .and_then(|uncovered| uncovered.get("executables"));
    for values in [uncovered, plan.get("unapproved_declared_executables")] {
        if let Some(items) = values.and_then(Value::as_array) {
            for item in items {
                if let Some(path) = item.as_str()
                    && !path.is_empty()
                {
                    paths.push(path.to_string());
                }
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

fn archive_args(
    game: &Game,
    archive: &Path,
    consent: &[&str],
    allow_native_injector: bool,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        "--game".into(),
        game.game_id.clone().into(),
        "install".into(),
        archive.as_os_str().into(),
    ];
    args.extend(consent.iter().map(OsString::from));
    args.extend(["--path".into(), game.path.clone().into_os_string()]);
    if allow_native_injector {
        args.push("--allow-native-injector".into());
    }
    args
}

#[tauri::command]
pub(crate) async fn uninstall(
    state: tauri::State<'_, AppState>,
    game_id: String,
    path: String,
    package_id: String,
    run_id: String,
) -> Result<Value, CommandError> {
    let backend = state.backend()?;
    let busy = Arc::clone(&state.busy);
    let game = installation(game_id, path);
    let _ = run_id;
    run_blocking(&busy, move || backend.uninstall(&game, &package_id)).await
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
