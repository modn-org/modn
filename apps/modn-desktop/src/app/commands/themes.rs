//! Theme listing, resolution, selection and import commands.
//!
//! Every action runs the configured CLI off the UI thread. A theme is
//! untrusted content, so `resolve_theme` also decides which directory the
//! `modn-theme` protocol may read; the protocol module enforces the file rules.

use super::{args, run_blocking};
use crate::app::error::CommandError;
use crate::app::state::AppState;
use crate::app::theme_assets::ASSET_BASE;
use serde_json::{Value, json};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri_plugin_dialog::DialogExt;

#[tauri::command]
pub(crate) async fn list_themes(
    state: tauri::State<'_, AppState>,
    game: Option<String>,
) -> Result<Value, CommandError> {
    let args = args::theme_list(game.as_deref())?;
    let backend = state.backend()?;
    let busy = Arc::clone(&state.busy);
    run_blocking(&busy, move || backend.call(&args)).await
}

/// Resolves the theme the desktop must apply.
///
/// The CLI answer names the theme directory. Its `assets_dir` never reaches
/// the webview: the handler replaces it with the stored protocol root and adds
/// `asset_base`. An invalid directory clears any earlier one. A newer request
/// supersedes this one: a stale answer never changes the root and reports
/// `{"stale": true, "game": "<game>"}` so the controller can discard it.
#[tauri::command]
pub(crate) async fn resolve_theme(
    state: tauri::State<'_, AppState>,
    game: String,
) -> Result<Value, CommandError> {
    let Some(args) = resolve_request(state.default_theme(), &game)? else {
        let ticket = state.next_theme_ticket();
        return Ok(if state.store_theme_root_if_latest(ticket, None) {
            default_theme_answer(&game)
        } else {
            stale_answer(&game)
        });
    };
    let ticket = state.next_theme_ticket();
    let backend = match state.backend() {
        Ok(backend) => backend,
        Err(error) => return resolve_failure(&state, ticket, &game, error),
    };
    let busy = Arc::clone(&state.busy);
    let value = match run_blocking(&busy, move || backend.call(&args)).await {
        Ok(value) => value,
        Err(error) => return resolve_failure(&state, ticket, &game, error),
    };
    let resolved = match resolved_theme(value) {
        Ok(resolved) => resolved,
        Err(error) => return resolve_failure(&state, ticket, &game, error),
    };
    Ok(if state.store_theme_root_if_latest(ticket, resolved.root) {
        resolved.answer
    } else {
        stale_answer(&game)
    })
}

/// Reports a resolve failure that happened after its ticket was taken.
///
/// The stored root belongs to the request that took the ticket, so this clears
/// it before the error reaches the webview. A newer request keeps its own root
/// and receives the stale answer instead.
fn resolve_failure(
    state: &AppState,
    ticket: u64,
    game: &str,
    error: CommandError,
) -> Result<Value, CommandError> {
    if state.store_theme_root_if_latest(ticket, None) {
        Err(error)
    } else {
        Ok(stale_answer(game))
    }
}

#[tauri::command]
pub(crate) async fn select_theme(
    state: tauri::State<'_, AppState>,
    game: String,
    theme: Option<String>,
    variant: Option<String>,
) -> Result<Value, CommandError> {
    let args = select_request(&game, theme.as_deref(), variant.as_deref())?;
    let backend = state.backend()?;
    let busy = Arc::clone(&state.busy);
    run_blocking(&busy, move || backend.call_with_inactivity_timeout(&args)).await
}

/// The exact select request: a named theme, or the explicit `--none` default.
///
/// A variant belongs to a named theme, so a variant without a theme is
/// refused before any CLI process starts.
pub(super) fn select_request(
    game: &str,
    theme: Option<&str>,
    variant: Option<&str>,
) -> Result<Vec<OsString>, CommandError> {
    match theme {
        Some(theme) => args::theme_select(game, theme, variant),
        None => {
            if variant.is_some() {
                return Err(CommandError::invalid_id());
            }
            args::theme_select_none(game)
        }
    }
}

#[tauri::command]
pub(crate) async fn reset_theme(
    state: tauri::State<'_, AppState>,
    game: Option<String>,
) -> Result<Value, CommandError> {
    let args = args::theme_reset(game.as_deref())?;
    let backend = state.backend()?;
    let busy = Arc::clone(&state.busy);
    run_blocking(&busy, move || backend.call_with_inactivity_timeout(&args)).await
}

/// Shows the native folder picker from Rust, then registers the chosen theme.
///
/// The webview never supplies or edits the folder path. A cancelled picker
/// answers `{"cancelled": true}` and spawns nothing.
#[tauri::command]
pub(crate) async fn import_theme(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Value, CommandError> {
    let picked = pick_theme_folder(&app).await?;
    match import_plan(picked.as_deref()) {
        ImportPlan::Cancelled => Ok(cancelled_answer()),
        ImportPlan::Register(args) => {
            let backend = state.backend()?;
            let busy = Arc::clone(&state.busy);
            run_blocking(&busy, move || backend.call_with_inactivity_timeout(&args)).await
        }
    }
}

/// A resolve answer ready for the webview plus the root the protocol serves.
#[derive(Debug)]
pub(super) struct ResolvedTheme {
    pub answer: Value,
    pub root: Option<PathBuf>,
}

/// The exact resolve request, or `None` when `--default-theme` answers without
/// a CLI process. Receiving `None` must never spawn the backend.
pub(super) fn resolve_request(
    default_theme: bool,
    game: &str,
) -> Result<Option<Vec<OsString>>, CommandError> {
    let args = args::theme_resolve(game)?;
    Ok(if default_theme { None } else { Some(args) })
}

/// The synthetic answer of a `--default-theme` start.
pub(super) fn default_theme_answer(game: &str) -> Value {
    json!({
        "game": game,
        "theme": Value::Null,
        "fallback": Value::Null,
        "disabled": "default-theme-flag",
        "diagnostics": [],
        "asset_base": ASSET_BASE,
    })
}

/// The answer of a resolve that a newer request superseded.
///
/// The root stays as the newer call set it. The controller discards this
/// answer, so it carries no theme data or `asset_base`.
pub(super) fn stale_answer(game: &str) -> Value {
    json!({"stale": true, "game": game})
}

/// Prepares one resolve answer for the webview.
///
/// `assets_dir` is removed at every depth. A theme needs an absolute path to
/// an existing directory; a null theme stores no root. A malformed answer is
/// refused so the webview falls back to the default theme.
pub(super) fn resolved_theme(mut value: Value) -> Result<ResolvedTheme, CommandError> {
    let Some(object) = value.as_object_mut() else {
        return Err(CommandError::theme_invalid());
    };
    let root = match object.get_mut("theme") {
        Some(Value::Object(theme)) => {
            let root = theme
                .remove("assets_dir")
                .and_then(|value| value.as_str().map(PathBuf::from));
            let Some(root) = root.filter(|root| root.is_absolute() && root.is_dir()) else {
                return Err(CommandError::theme_invalid());
            };
            Some(root)
        }
        Some(Value::Null) | None => None,
        Some(_) => return Err(CommandError::theme_invalid()),
    };
    object.insert(
        "asset_base".to_owned(),
        Value::String(ASSET_BASE.to_owned()),
    );
    Ok(ResolvedTheme {
        answer: value,
        root,
    })
}

/// What `import_theme` does after the native picker returns.
#[derive(Debug)]
pub(super) enum ImportPlan {
    Cancelled,
    Register(Vec<OsString>),
}

/// Turns the picker result into the cancelled answer or the register request.
pub(super) fn import_plan(folder: Option<&Path>) -> ImportPlan {
    match folder {
        None => ImportPlan::Cancelled,
        Some(folder) => ImportPlan::Register(args::theme_register(folder)),
    }
}

/// The answer of a cancelled import.
pub(super) fn cancelled_answer() -> Value {
    json!({"cancelled": true})
}

async fn pick_theme_folder(app: &tauri::AppHandle) -> Result<Option<PathBuf>, CommandError> {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Choose a theme folder")
            .blocking_pick_folder()
            .and_then(|file| file.into_path().ok())
    })
    .await
    .map_err(|_| CommandError::backend_failed())
}

#[cfg(test)]
mod tests;
