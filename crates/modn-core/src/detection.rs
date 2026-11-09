//! Find installed games: match store libraries against module store identities.
//!
//! A store library only proposes a folder. The game module's existing
//! `validate` policy confirms it, with that folder as its only readable root.
//! Detection reads libraries and game folders; it writes nothing.

use crate::game_modules::{policy, store_identities, StoreIdentities};
use crate::stores::{StoreInstallation, StoreLibraries, Storefront};
use serde::Serialize;
use std::path::PathBuf;

#[cfg(test)]
mod tests;

/// Store installation that the game module confirmed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct DetectedInstallation {
    /// Game whose module confirmed the folder.
    pub game_id: String,
    /// Store whose library lists the folder.
    pub store: Storefront,
    /// Store identity that matched the module declaration.
    pub store_id: String,
    /// Canonical game folder.
    pub path: PathBuf,
}

/// Store installation that matched a store identity but was not confirmed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct RefusedInstallation {
    /// Game whose store identity matched.
    pub game_id: String,
    /// Store whose library lists the folder.
    pub store: Storefront,
    /// Store identity that matched the module declaration.
    pub store_id: String,
    /// Folder as the library records it.
    pub path: PathBuf,
    /// Why Modn did not confirm the folder.
    pub reason: String,
}

/// Game whose module could not be read, so detection skipped it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct SkippedGame {
    /// Requested game.
    pub game_id: String,
    /// Module error.
    pub reason: String,
}

/// Result of one detection run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Detection {
    /// Confirmed installations, sorted by game and folder.
    pub confirmed: Vec<DetectedInstallation>,
    /// Matched folders that the module did not confirm.
    pub refused: Vec<RefusedInstallation>,
    /// Games whose module could not be read.
    pub skipped: Vec<SkippedGame>,
}

/// Detect installations of `games` in `libraries`.
///
/// Each folder that a library lists under a declared store identity is
/// confirmed through the game module, limited to that folder. The same
/// folder listed by two stores is reported once.
#[must_use]
pub fn detect_installations(libraries: &StoreLibraries, games: &[String]) -> Detection {
    let installations = libraries.installations();
    let mut detection = Detection::default();
    for game in games {
        match store_identities(game) {
            Ok(identities) => {
                confirm_matches(game, &identities, &installations, &mut detection);
            }
            Err(error) => detection.skipped.push(SkippedGame {
                game_id: game.clone(),
                reason: format!("{error:#}"),
            }),
        }
    }
    detection
        .confirmed
        .sort_by(|left, right| (&left.game_id, &left.path).cmp(&(&right.game_id, &right.path)));
    detection
        .confirmed
        .dedup_by(|right, left| left.game_id == right.game_id && left.path == right.path);
    detection
}

fn confirm_matches(
    game: &str,
    identities: &StoreIdentities,
    installations: &[StoreInstallation],
    detection: &mut Detection,
) {
    for installation in installations {
        if identities.get(installation.store) != Some(installation.store_id.as_str()) {
            continue;
        }
        match confirm(game, installation) {
            Ok(path) => detection.confirmed.push(DetectedInstallation {
                game_id: game.to_owned(),
                store: installation.store,
                store_id: installation.store_id.clone(),
                path,
            }),
            Err(reason) => detection.refused.push(RefusedInstallation {
                game_id: game.to_owned(),
                store: installation.store,
                store_id: installation.store_id.clone(),
                path: installation.path.clone(),
                reason,
            }),
        }
    }
}

fn confirm(game: &str, installation: &StoreInstallation) -> Result<PathBuf, String> {
    let path = installation
        .path
        .canonicalize()
        .map_err(|error| format!("cannot access the folder: {error}"))?;
    match policy::validate(game, &path) {
        Ok((true, _)) => Ok(path),
        Ok((false, marker)) => Err(format!("the folder does not contain {marker}")),
        Err(error) => Err(format!("the game module failed: {error:#}")),
    }
}
