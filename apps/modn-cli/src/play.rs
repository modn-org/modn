use crate::selection::selected_installation;
use crate::state_access::ManagerState;
use anyhow::Result;
use modn_core::detection::detect_installations;
use modn_core::error::{coded, ErrorCode};
use modn_core::launch::{execute_launch, plan_launch, system_launch, LaunchTarget};
use modn_core::stores::StoreLibraries;
use std::path::{Path, PathBuf};

pub(super) fn run(
    game: &str,
    path: Option<PathBuf>,
    state_path: &Path,
    requested: Option<&str>,
    json: bool,
) -> Result<()> {
    if requested.is_some_and(|id| id != game) {
        return Err(coded(
            ErrorCode::LaunchGameMismatch,
            "LAUNCH_GAME_MISMATCH: --game must match the play game ID",
        ));
    }
    let state = ManagerState::load_deferred(state_path)?;
    let (game_id, installation) = selected_installation(state.existing(), path, Some(game))
        .map_err(|error| {
            coded(
                ErrorCode::LaunchInstallationInvalid,
                format!("LAUNCH_INSTALLATION_INVALID: {error:#}"),
            )
        })?;
    if game_id != game {
        return Err(coded(
            ErrorCode::LaunchGameMismatch,
            format!("LAUNCH_GAME_MISMATCH: selected installation is registered as {game_id}, not {game}"),
        ));
    }
    let registered = state
        .existing()
        .map(modn_core::state::Store::list)
        .transpose()?
        .unwrap_or_default()
        .into_iter()
        .any(|entry| entry.game_id == game && entry.path == installation);
    if !registered {
        return Err(coded(
            ErrorCode::LaunchInstallationUnregistered,
            format!(
                "LAUNCH_INSTALLATION_UNREGISTERED: register it with `modn game add {game} --path <installation>`"
            ),
        ));
    }
    let detection = detect_installations(&StoreLibraries::system(), &[game.to_owned()]);
    let plan = plan_launch(state.existing(), game, &installation, &detection.confirmed)?;
    if !json {
        for warning in &plan.warnings {
            eprintln!("warning: {warning}");
        }
    }
    execute_launch(&plan, system_launch)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&plan)?);
    } else {
        match &plan.target {
            LaunchTarget::StoreUri { store, uri } => {
                println!("Launched {} via {}: {uri}", plan.game_id, store.as_str())
            }
            LaunchTarget::Executable { path, .. } => {
                println!("Launched {}: {}", plan.game_id, path.display());
            }
            other => println!("Launched {}: {other:?}", plan.game_id),
        }
    }
    Ok(())
}
