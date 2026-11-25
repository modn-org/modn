use super::args;
use super::{ensure_not_cancelled, installation, run_blocking, run_write};
use crate::app::error::{CommandError, plan_blocked};
use crate::app::state::{AppState, Busy};
use crate::backend::{Backend, Game, Review, reviewable};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[tauri::command]
#[expect(
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
    allow_executables: Vec<String>,
    run_id: String,
) -> Result<Value, CommandError> {
    let run = state.runs.start(&run_id)?;
    let backend = state.backend()?;
    let game = installation(game_id, path);
    let source = PathBuf::from(archive);
    let review = run_review(&state.busy, &backend, &game, &source).await?;
    ensure_not_cancelled(&run)?;

    let plan = archive_plan(
        &state.busy,
        &backend,
        &game,
        &review,
        allow_native_injector,
        &allow_executables,
    )
    .await?;
    ensure_not_cancelled(&run)?;
    if !reviewable(&plan) {
        if let Some(candidates) = executable_approval(&plan, &allow_executables) {
            let mut vetted = allow_executables.clone();
            vetted.extend(candidates.iter().cloned());
            let plan = archive_dry_run(
                &state.busy,
                &backend,
                &game,
                &review.archive,
                allow_native_injector,
                &vetted,
            )
            .await?;
            ensure_not_cancelled(&run)?;
            if reviewable(&plan) {
                return Err(CommandError::approval_required(false, candidates));
            }
        }
        return Err(plan_blocked().into());
    }

    let commit = args::install_archive_commit(
        &game,
        &review.archive,
        allow_native_injector,
        &allow_executables,
    )?;

    let fresh = run_review(&state.busy, &backend, &game, &source).await?;
    ensure_not_cancelled(&run)?;
    ensure_review_unchanged(&review.plan, &fresh.plan)?;
    if plan != review.plan {
        let fresh_plan = archive_dry_run(
            &state.busy,
            &backend,
            &game,
            &review.archive,
            allow_native_injector,
            &allow_executables,
        )
        .await?;
        ensure_not_cancelled(&run)?;
        ensure_plan_unchanged(&plan, &fresh_plan)?;
    }
    ensure_not_cancelled(&run)?;
    run_write(&app, &state, run, commit).await
}

async fn run_review(
    busy: &Arc<Busy>,
    backend: &Backend,
    game: &Game,
    archive: &Path,
) -> Result<Review, CommandError> {
    let backend = backend.clone();
    let game = game.clone();
    let archive = archive.to_path_buf();
    run_blocking(busy, move || backend.review(&game, &archive)).await
}

async fn archive_dry_run(
    busy: &Arc<Busy>,
    backend: &Backend,
    game: &Game,
    archive: &Path,
    allow_native_injector: bool,
    executables: &[String],
) -> Result<Value, CommandError> {
    let args = args::install_archive_dry_run(game, archive, allow_native_injector, executables)?;
    let backend = backend.clone();
    run_blocking(busy, move || backend.call(&args)).await
}

async fn archive_plan(
    busy: &Arc<Busy>,
    backend: &Backend,
    game: &Game,
    review: &Review,
    allow_native_injector: bool,
    executables: &[String],
) -> Result<Value, CommandError> {
    if allow_native_injector || !executables.is_empty() {
        return archive_dry_run(
            busy,
            backend,
            game,
            &review.archive,
            allow_native_injector,
            executables,
        )
        .await;
    }
    Ok(review.plan.clone())
}

fn executable_approval(plan: &Value, approved: &[String]) -> Option<Vec<String>> {
    let blockers = plan["blockers"].as_array()?;
    let mut candidates = BTreeSet::new();
    for blocker in blockers {
        match blocker["code"].as_str()? {
            "EXECUTABLE_PAYLOAD" => {}
            "NO_INSTALLABLE_FILES" => continue,
            _ => return None,
        }
        let path = executable_path(blocker["message"].as_str()?)?;
        if approved.iter().any(|approved| approved == path) {
            return None;
        }
        candidates.insert(path.to_string());
    }
    if candidates.is_empty() {
        return None;
    }
    Some(candidates.into_iter().collect())
}

fn executable_path(message: &str) -> Option<&str> {
    let path = message
        .strip_prefix("Executable payload: ")
        .or_else(|| message.strip_prefix("Executable target: "))?;
    let tool = path.ends_with(".exe") && path.split('/').any(|part| part == "tools");
    (tool && args::game_relative(path)).then_some(path)
}

fn ensure_review_unchanged(reviewed: &Value, fresh: &Value) -> Result<(), CommandError> {
    match (
        reviewed["archive_blake3"].as_str(),
        fresh["archive_blake3"].as_str(),
    ) {
        (Some(reviewed), Some(fresh)) if reviewed == fresh => {}
        _ => return Err(CommandError::digest_mismatch()),
    }
    ensure_plan_unchanged(reviewed, fresh)
}

fn ensure_plan_unchanged(plan: &Value, fresh: &Value) -> Result<(), CommandError> {
    if fresh != plan {
        return Err(CommandError::plan_changed());
    }
    Ok(())
}

#[tauri::command]
#[expect(
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
    allow_executables: Vec<String>,
    run_id: String,
) -> Result<Value, CommandError> {
    let run = state.runs.start(&run_id)?;
    let game = installation(game_id, path);
    let commit =
        args::install_mod_commit(&game, &mod_id, allow_native_injector, &allow_executables)?;
    run_write(&app, &state, run, commit).await
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
    let run = state.runs.start(&run_id)?;
    let commit = args::uninstall(&installation(game_id, path), &package_id);
    run_write(&app, &state, run, commit).await
}

#[tauri::command]
#[expect(
    clippy::too_many_arguments,
    reason = "The webview contract names every approval and run field explicitly"
)]
pub(crate) async fn update(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    game_id: String,
    path: String,
    package_id: String,
    allow_native_injector: bool,
    allow_executables: Vec<String>,
    run_id: String,
) -> Result<Value, CommandError> {
    let run = state.runs.start(&run_id)?;
    let backend = state.backend()?;
    let game = installation(game_id, path);
    let list = args::list_packages(&game);
    let packages = run_blocking(&state.busy, move || backend.call(&list)).await?;
    ensure_not_cancelled(&run)?;
    let commit = args::update(
        &game,
        &packages,
        &package_id,
        allow_native_injector,
        &allow_executables,
    )?;
    ensure_not_cancelled(&run)?;
    run_write(&app, &state, run, commit).await
}

#[tauri::command]
pub(crate) fn cancel_run(
    state: tauri::State<'_, AppState>,
    run_id: String,
) -> Result<(), CommandError> {
    state.runs.cancel(&run_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ensure_plan_unchanged, ensure_review_unchanged, executable_approval};
    use serde_json::{Value, json};

    fn blocked(message: &str) -> Value {
        json!({
            "status": "blocked",
            "metadata_only": true,
            "operations": [],
            "blockers": [{"code": "EXECUTABLE_PAYLOAD", "message": message}],
        })
    }

    fn reviewed(digest: &str, status: &str) -> Value {
        json!({"status": status, "archive_blake3": digest})
    }

    #[test]
    fn a_payload_blocker_names_the_exact_path_to_approve() {
        assert_eq!(
            executable_approval(&blocked("Executable payload: tools/helper.exe"), &[]),
            Some(vec!["tools/helper.exe".to_string()])
        );
        assert_eq!(
            executable_approval(&blocked("Executable target: mods/tools/run.exe"), &[]),
            Some(vec!["mods/tools/run.exe".to_string()])
        );
    }

    #[test]
    fn a_derived_no_installable_files_blocker_keeps_the_executable_approval() {
        let plan = json!({
            "status": "blocked",
            "metadata_only": true,
            "operations": [],
            "blockers": [
                {"code": "EXECUTABLE_PAYLOAD", "message": "Executable payload: tools/helper.exe"},
                {"code": "NO_INSTALLABLE_FILES", "message": "No files selected"},
            ],
        });
        assert_eq!(
            executable_approval(&plan, &[]),
            Some(vec!["tools/helper.exe".to_string()])
        );
        let derived_only = json!({
            "status": "blocked",
            "metadata_only": true,
            "operations": [],
            "blockers": [{"code": "NO_INSTALLABLE_FILES", "message": "No files selected"}],
        });
        assert_eq!(
            executable_approval(&derived_only, &[]),
            None,
            "a plan with no executable path offers nothing to approve"
        );
    }

    #[test]
    fn permanent_and_unsafe_blockers_never_offer_an_approval() {
        let conflict = json!({
            "status": "blocked",
            "metadata_only": true,
            "operations": [],
            "blockers": [{"code": "CONFLICT", "message": "Another package owns these files"}],
        });
        assert_eq!(executable_approval(&conflict, &[]), None);

        let mixed = json!({
            "status": "blocked",
            "metadata_only": true,
            "operations": [],
            "blockers": [
                {"code": "EXECUTABLE_PAYLOAD", "message": "Executable payload: tools/helper.exe"},
                {"code": "CONFLICT", "message": "Another package owns these files"},
            ],
        });
        assert_eq!(executable_approval(&mixed, &[]), None);

        for message in [
            "Executable payload: helper.exe",
            "Executable payload: tools/../helper.exe",
            "Executable payload: C:/tools/helper.exe",
            "Executable payload: /tools/helper.exe",
            "Executable payload: -tools/helper.exe",
            "Executable payload: ",
            "Executable payload: tools/helper.txt",
            "Something else",
        ] {
            assert_eq!(
                executable_approval(&blocked(message), &[]),
                None,
                "{message}"
            );
        }
    }

    #[test]
    fn an_already_approved_path_does_not_ask_again() {
        assert_eq!(
            executable_approval(
                &blocked("Executable payload: tools/helper.exe"),
                &["tools/helper.exe".to_string()]
            ),
            None
        );
    }

    #[test]
    fn the_review_recheck_requires_the_archive_blake3_pin_in_both_plans() {
        let pin = "a".repeat(64);
        let other = "b".repeat(64);
        let plan = reviewed(&pin, "reviewable");
        ensure_review_unchanged(&plan, &reviewed(&pin, "reviewable"))
            .expect("the same BLAKE3 pin may write");
        assert_eq!(
            ensure_review_unchanged(&plan, &reviewed(&other, "reviewable"))
                .expect_err("a changed BLAKE3 pin is refused")
                .code,
            "DIGEST_MISMATCH"
        );
        let without_pin = json!({"status": "reviewable"});
        assert_eq!(
            ensure_review_unchanged(&plan, &without_pin)
                .expect_err("a fresh plan without the pin is refused")
                .code,
            "DIGEST_MISMATCH"
        );
        assert_eq!(
            ensure_review_unchanged(&without_pin, &plan)
                .expect_err("a reviewed plan without the pin is refused")
                .code,
            "DIGEST_MISMATCH"
        );
        let numeric_pin = json!({"status": "reviewable", "archive_blake3": 7});
        assert_eq!(
            ensure_review_unchanged(&plan, &numeric_pin)
                .expect_err("a non-string pin is refused")
                .code,
            "DIGEST_MISMATCH"
        );
    }

    #[test]
    fn the_review_recheck_refuses_a_changed_digest_or_plan() {
        let plan = reviewed("first", "reviewable");
        ensure_review_unchanged(&plan, &reviewed("first", "reviewable"))
            .expect("the same review may write");
        assert_eq!(
            ensure_review_unchanged(&plan, &reviewed("second", "reviewable"))
                .expect_err("a changed archive is refused")
                .code,
            "DIGEST_MISMATCH"
        );
        assert_eq!(
            ensure_review_unchanged(&plan, &reviewed("first", "blocked"))
                .expect_err("a changed plan is refused")
                .code,
            "PLAN_BLOCKED"
        );
    }

    #[test]
    fn the_approved_plan_recheck_refuses_a_changed_plan() {
        let plan = reviewed("first", "reviewable");
        ensure_plan_unchanged(&plan, &plan.clone()).expect("an unchanged plan may write");
        assert_eq!(
            ensure_plan_unchanged(&plan, &reviewed("second", "reviewable"))
                .expect_err("a changed approved plan is refused")
                .code,
            "PLAN_BLOCKED"
        );
    }
}
