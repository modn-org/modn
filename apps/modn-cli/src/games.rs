//! `modn game add/list/remove`: game registrations only; never game files.
use crate::commands::GameCommand;
use crate::selection::choose_candidate;
use crate::state_access::{ManagerState, ProtectedTarget};
use anyhow::{bail, Result};
use modn_core::state::Store;
use std::path::{Path, PathBuf};

pub(super) fn run(
    command: GameCommand,
    state_path: &Path,
    requested: Option<&str>,
    json: bool,
) -> Result<()> {
    if let GameCommand::Add { game, .. } | GameCommand::Remove { game, .. } = &command {
        if requested.is_some_and(|id| id != game) {
            bail!("--game must match the game ID passed to game add/remove");
        }
    }
    match command {
        GameCommand::Add { game, path } => add(&game, path, state_path, json),
        GameCommand::Remove { game, path } => remove(&game, path.as_deref(), state_path, json),
        GameCommand::List => list(state_path, json),
    }
}

fn add(game: &str, path: Option<PathBuf>, state_path: &Path, json: bool) -> Result<()> {
    let state = ManagerState::load(state_path)?;
    let candidate = choose_candidate(game, path.clone())?;
    let store = state.open_writable(&ProtectedTarget {
        game: Some(game.to_owned()),
        root: Some(path.unwrap_or_else(|| candidate.path.clone())),
    })?;
    let entry = store.add_game(game, &candidate.path, &candidate.source)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&entry)?);
    } else {
        println!("Registered {} ({})", entry.path.display(), entry.source);
    }
    Ok(())
}

fn remove(game: &str, path: Option<&Path>, state_path: &Path, json: bool) -> Result<()> {
    let mut store = ManagerState::load(state_path)?
        .require_existing("no game registrations to remove")?
        .open_writable(&ProtectedTarget {
            game: Some(game.to_owned()),
            root: None,
        })?;
    let removed = store.remove_game(game, path)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&removed)?);
    } else {
        println!(
            "Unregistered {} ({})",
            removed.game_id,
            removed.path.display()
        );
    }
    Ok(())
}

// Listing reads existing registrations and never creates a database.
fn list(state_path: &Path, json: bool) -> Result<()> {
    let entries = ManagerState::load(state_path)?
        .into_existing()
        .as_ref()
        .map(Store::list)
        .transpose()?
        .unwrap_or_default();
    if json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
    } else if entries.is_empty() {
        println!("No game installations registered.");
    } else {
        for entry in entries {
            println!(
                "{}: {} ({})",
                entry.game_id,
                entry.path.display(),
                entry.source
            );
        }
    }
    Ok(())
}
