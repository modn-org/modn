use crate::commands::GameCommand;
use crate::game_detect;
use crate::selection::choose_candidate;
use crate::state_access::{ManagerState, ProtectedTarget};
use anyhow::{bail, Result};
use modn_core::manager_roots::ManagerRoots;
use modn_core::state::Store;
use std::path::{Path, PathBuf};

pub(super) fn run(
    command: GameCommand,
    roots: &ManagerRoots,
    requested: Option<&str>,
    json: bool,
) -> Result<()> {
    if let GameCommand::Add { game, .. } | GameCommand::Remove { game, .. } = &command {
        if requested.is_some_and(|id| id != game) {
            bail!("--game must match the game ID passed to game add/remove");
        }
    }
    match command {
        GameCommand::Add { game, path } => add(roots, &game, path, json),
        GameCommand::Remove { game, path } => remove(roots, &game, path.as_deref(), json),
        GameCommand::List => list(roots, json),
        GameCommand::Detect { register } => game_detect::run(roots, requested, register, json),
    }
}

fn add(roots: &ManagerRoots, game: &str, path: Option<PathBuf>, json: bool) -> Result<()> {
    let state = ManagerState::load(roots)?;
    let candidate = choose_candidate(roots, game, path.clone())?;
    let store = state.open_writable(&ProtectedTarget {
        game: Some(game.to_owned()),
        root: Some(path.unwrap_or_else(|| candidate.path.clone())),
    })?;
    let entry = store.add_game(roots, game, &candidate.path, &candidate.source)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&entry)?);
    } else {
        println!("Registered {} ({})", entry.path.display(), entry.source);
    }
    Ok(())
}

fn remove(roots: &ManagerRoots, game: &str, path: Option<&Path>, json: bool) -> Result<()> {
    let mut store = ManagerState::load(roots)?
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

fn list(roots: &ManagerRoots, json: bool) -> Result<()> {
    let entries = ManagerState::load(roots)?
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
