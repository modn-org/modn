//! Removal of one registered theme.

use super::super::error::{Result, ThemeError};
use super::super::limits::is_valid_id;
use super::super::storage;
use super::layout;
use super::{check_id, RemovedTheme};
use std::fs;
use std::io;
use std::path::Path;

/// Remove one registration below one theme root.
pub(super) fn remove(root: &Path, id: &str, game: Option<&str>) -> Result<RemovedTheme> {
    check_id(id)?;
    if let Some(game) = game {
        check_id(game)?;
    }
    let registry = layout::registry_root(root);
    let _lock = storage::lock(root)?;
    let mut games = match game {
        Some(game) => {
            let entry = layout::entry_root(&registry, game, id);
            match fs::symlink_metadata(&entry) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    return Err(ThemeError::NotRegistered { id: id.to_owned() });
                }
                Err(error) => {
                    return Err(ThemeError::io(
                        "cannot inspect the theme registration",
                        error,
                    ));
                }
                Ok(metadata) if !metadata.file_type().is_dir() => {
                    return Err(ThemeError::corrupt(
                        &entry,
                        "a theme registration must be a real directory",
                    ));
                }
                Ok(_) => {}
            }
            vec![game.to_owned()]
        }
        None => games_for(id, &registry)?,
    };
    match games.len() {
        0 => Err(ThemeError::NotRegistered { id: id.to_owned() }),
        1 => {
            let game = games.remove(0);
            let entry = layout::entry_root(&registry, &game, id);
            fs::remove_dir_all(&entry)
                .map_err(|error| ThemeError::io("cannot remove the theme registration", error))?;
            Ok(RemovedTheme {
                game,
                id: id.to_owned(),
            })
        }
        _ => Err(ThemeError::Ambiguous {
            id: id.to_owned(),
            games,
        }),
    }
}

/// Return the games that register `id`, sorted.
fn games_for(id: &str, registry: &Path) -> Result<Vec<String>> {
    let entries = match fs::read_dir(registry) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(ThemeError::io("cannot list the theme registry", error)),
        Ok(entries) => entries,
    };
    let mut games = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|error| ThemeError::io("cannot list the theme registry", error))?;
        if !entry
            .file_type()
            .map_err(|error| ThemeError::io("cannot inspect a theme registry entry", error))?
            .is_dir()
        {
            continue;
        }
        let Some(game) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !is_valid_id(&game) {
            continue;
        }
        match fs::symlink_metadata(entry.path().join(id)) {
            Ok(metadata) if metadata.file_type().is_dir() => games.push(game),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(ThemeError::io(
                    "cannot inspect the theme registration",
                    error,
                ));
            }
            Ok(_) => {}
        }
    }
    games.sort();
    Ok(games)
}
