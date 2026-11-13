//! Game registration and launch commands.

use super::{args, run_blocking};
use crate::app::error::CommandError;
use crate::app::state::AppState;
use crate::backend::Game;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

fn installations_of(games: Vec<Game>, game_id: &str) -> Vec<Game> {
    games
        .into_iter()
        .filter(|game| game.game_id == game_id)
        .collect()
}

#[tauri::command]
pub(crate) async fn list_games(state: tauri::State<'_, AppState>) -> Result<Value, CommandError> {
    let backend = state.backend()?;
    let busy = Arc::clone(&state.busy);
    run_blocking(&busy, move || {
        let games: Vec<Game> = serde_json::from_value(backend.call(&args::list_games())?)?;
        Ok(serde_json::to_value(games)?)
    })
    .await
}

#[tauri::command]
pub(crate) async fn list_installations(
    state: tauri::State<'_, AppState>,
    game_id: String,
) -> Result<Value, CommandError> {
    let backend = state.backend()?;
    let busy = Arc::clone(&state.busy);
    run_blocking(&busy, move || {
        let games: Vec<Game> = serde_json::from_value(backend.call(&args::list_games())?)?;
        Ok(serde_json::to_value(installations_of(games, &game_id))?)
    })
    .await
}

#[tauri::command]
pub(crate) async fn detect_games(state: tauri::State<'_, AppState>) -> Result<Value, CommandError> {
    let backend = state.backend()?;
    let busy = Arc::clone(&state.busy);
    run_blocking(&busy, move || backend.call(&args::detect_games())).await
}

#[tauri::command]
pub(crate) async fn activate_game(
    state: tauri::State<'_, AppState>,
    game_id: String,
    path: String,
) -> Result<Value, CommandError> {
    let backend = state.backend()?;
    let busy = Arc::clone(&state.busy);
    let args = args::game_add(&game_id, Path::new(&path));
    run_blocking(&busy, move || backend.call(&args)).await
}

/// Shows the native folder picker from Rust, then registers the chosen folder.
/// The webview never supplies or edits the folder path.
#[tauri::command]
pub(crate) async fn add_game(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    game_id: String,
) -> Result<Option<Value>, CommandError> {
    let Some(folder) = pick_folder(&app).await? else {
        return Ok(None);
    };
    let backend = state.backend()?;
    let busy = Arc::clone(&state.busy);
    let args = args::game_add(&game_id, &folder);
    let game = run_blocking(&busy, move || backend.call(&args)).await?;
    Ok(Some(game))
}

/// Asks in a native dialog before unregistering. No removes a run.
#[tauri::command]
pub(crate) async fn remove_game(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    game_id: String,
    path: String,
) -> Result<Option<Value>, CommandError> {
    if !confirm_removal(&app, &game_id).await? {
        return Ok(None);
    }
    let backend = state.backend()?;
    let busy = Arc::clone(&state.busy);
    let args = args::game_remove(&game_id, Path::new(&path));
    let removed = run_blocking(&busy, move || backend.call(&args)).await?;
    Ok(Some(removed))
}

#[tauri::command]
pub(crate) async fn play(
    state: tauri::State<'_, AppState>,
    game_id: String,
    path: String,
) -> Result<Value, CommandError> {
    let backend = state.backend()?;
    let busy = Arc::clone(&state.busy);
    let args = args::play(&game_id, Path::new(&path));
    run_blocking(&busy, move || backend.call(&args)).await
}

async fn pick_folder(app: &tauri::AppHandle) -> Result<Option<PathBuf>, CommandError> {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Choose the game folder")
            .blocking_pick_folder()
            .and_then(|file| file.into_path().ok())
    })
    .await
    .map_err(|_| CommandError::backend_failed())
}

async fn confirm_removal(app: &tauri::AppHandle, game_id: &str) -> Result<bool, CommandError> {
    let app = app.clone();
    let game_id = game_id.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .message(format!(
                "Remove {game_id} from Modn? Modn only unregisters this installation; every game file stays on disk."
            ))
            .title("Modn")
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::YesNo)
            .blocking_show()
    })
    .await
    .map_err(|_| CommandError::backend_failed())
}

#[cfg(test)]
mod tests {
    use super::installations_of;
    use crate::backend::Game;
    use std::path::PathBuf;

    #[test]
    fn list_installations_selects_one_game_from_the_registration_list() {
        let games = vec![
            Game {
                game_id: "one".into(),
                path: PathBuf::from(r"C:\games\one"),
            },
            Game {
                game_id: "two".into(),
                path: PathBuf::from(r"C:\games\two"),
            },
            Game {
                game_id: "one".into(),
                path: PathBuf::from(r"D:\games\one"),
            },
        ];
        let selected = installations_of(games, "one");
        assert_eq!(selected.len(), 2);
        assert!(selected.iter().all(|game| game.game_id == "one"));
        assert!(installations_of(Vec::new(), "missing").is_empty());
    }
}
