//! Write commands: every one streams progress and can be cancelled before the
//! CLI's commit point.

use super::args;
use super::{installation, run_blocking, run_write};
use crate::app::error::{CommandError, plan_blocked};
use crate::app::state::AppState;
use crate::backend::reviewable;
use serde_json::Value;
use std::path::PathBuf;

#[tauri::command]
#[allow(
    clippy::too_many_arguments,
    reason = "The webview contract names every approval and run field explicitly"
)]
pub(crate) async fn install_archive(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    game_id: String,
    path: String,
    archive: String,
    allow_native_injector: bool,
    allow_executable: bool,
    run_id: String,
) -> Result<Value, CommandError> {
    let backend = state.backend()?;
    let game = installation(game_id, path);
    let archive = PathBuf::from(archive);
    let (plan, archive) = if allow_native_injector || allow_executable {
        let dry_run = args::install_archive_dry_run(&game, &archive, allow_native_injector);
        let backend = backend.clone();
        let plan = run_blocking(&state.busy, move || backend.call(&dry_run)).await?;
        (plan, archive)
    } else {
        let backend = backend.clone();
        let reviewed_game = game.clone();
        let reviewed_archive = archive.clone();
        let review = run_blocking(&state.busy, move || {
            backend.review(&reviewed_game, &reviewed_archive)
        })
        .await?;
        (review.plan, review.archive)
    };
    if !reviewable(&plan) {
        return Err(plan_blocked().into());
    }
    let executables = if allow_executable {
        declared_executables(&plan)
    } else {
        Vec::new()
    };
    let commit = args::install_archive_commit(&game, &archive, allow_native_injector, &executables);
    run_write(&app, &state, run_id, commit).await
}

#[tauri::command]
#[allow(
    clippy::too_many_arguments,
    reason = "The webview contract names every approval and run field explicitly"
)]
pub(crate) async fn install_mod(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    game_id: String,
    path: String,
    mod_id: String,
    allow_native_injector: bool,
    allow_executable: bool,
    run_id: String,
) -> Result<Value, CommandError> {
    let backend = state.backend()?;
    let game = installation(game_id, path);
    let executables = if allow_executable {
        let dry_run = args::install_mod_dry_run(&game, &mod_id, allow_native_injector);
        let backend = backend.clone();
        let plan = run_blocking(&state.busy, move || backend.call(&dry_run)).await?;
        declared_executables(&plan)
    } else {
        Vec::new()
    };
    let commit = args::install_mod_commit(&game, &mod_id, allow_native_injector, &executables);
    run_write(&app, &state, run_id, commit).await
}

#[tauri::command]
pub(crate) async fn uninstall(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    game_id: String,
    path: String,
    package_id: String,
    run_id: String,
) -> Result<Value, CommandError> {
    let commit = args::uninstall(&installation(game_id, path), &package_id);
    run_write(&app, &state, run_id, commit).await
}

#[tauri::command]
pub(crate) async fn update(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    game_id: String,
    path: String,
    package_id: String,
    run_id: String,
) -> Result<Value, CommandError> {
    let commit = args::update(&installation(game_id, path), &package_id);
    run_write(&app, &state, run_id, commit).await
}

/// Cancels a running write. An unknown run id is a no-op.
#[tauri::command]
pub(crate) fn cancel_run(
    state: tauri::State<'_, AppState>,
    run_id: String,
) -> Result<(), CommandError> {
    state.runs.cancel(&run_id);
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::declared_executables;
    use serde_json::json;

    #[test]
    fn executables_come_from_both_plan_shapes_without_duplicates() {
        let plan = json!({
            "approval": {"uncovered": {"executables": ["bin/b.exe", "bin/a.exe"]}},
            "unapproved_declared_executables": ["bin/a.exe", "bin/c.exe"]
        });
        assert_eq!(
            declared_executables(&plan),
            vec!["bin/a.exe", "bin/b.exe", "bin/c.exe"]
        );
        assert!(declared_executables(&json!({})).is_empty());
    }
}
