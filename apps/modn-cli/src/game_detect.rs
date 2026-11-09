use crate::state_access::{validate_state_path, ManagerState, ProtectedTarget};
use anyhow::Result;
use modn_core::detection::{detect_installations, DetectedInstallation, Detection};
use modn_core::game_modules::available_games;
use modn_core::state::Store;
use modn_core::stores::StoreLibraries;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    New,
    Registered,
    AlreadyRegistered,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Registered => "registered",
            Self::AlreadyRegistered => "already_registered",
        }
    }
}

pub(super) fn run(
    state_path: &Path,
    requested: Option<&str>,
    register: bool,
    json: bool,
) -> Result<()> {
    let games = match requested {
        Some(game) => vec![game.to_owned()],
        None => available_games()?,
    };
    let detection = detect_installations(&StoreLibraries::system(), &games);
    let state = ManagerState::load(state_path)?;
    let registered: Vec<PathBuf> = state
        .existing()
        .map(Store::list)
        .transpose()?
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| entry.path.canonicalize().ok())
        .collect();
    let mut statuses: Vec<_> = detection
        .confirmed
        .iter()
        .map(|found| {
            if registered.contains(&found.path) {
                Status::AlreadyRegistered
            } else {
                Status::New
            }
        })
        .collect();
    if register {
        register_new(state, state_path, &registered, &detection, &mut statuses)?;
    }
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report(&detection, &statuses))?
        );
    } else {
        print_human(&detection, &statuses);
    }
    Ok(())
}

fn register_new(
    state: ManagerState,
    state_path: &Path,
    registered: &[PathBuf],
    detection: &Detection,
    statuses: &mut [Status],
) -> Result<()> {
    let new: Vec<(usize, &DetectedInstallation)> = detection
        .confirmed
        .iter()
        .enumerate()
        .filter(|(index, _)| statuses[*index] == Status::New)
        .collect();
    let Some((_, first)) = new.first() else {
        return Ok(());
    };
    for (_, found) in &new {
        validate_state_path(
            state_path,
            Some(&found.game_id),
            Some(&found.path),
            registered,
        )?;
    }
    let store = state.open_writable(&ProtectedTarget {
        game: Some(first.game_id.clone()),
        root: Some(first.path.clone()),
    })?;
    for (index, found) in new {
        store.add_game(&found.game_id, &found.path, found.store.as_str())?;
        statuses[index] = Status::Registered;
    }
    Ok(())
}

fn report(detection: &Detection, statuses: &[Status]) -> Value {
    let installations: Vec<Value> = detection
        .confirmed
        .iter()
        .zip(statuses)
        .map(|(found, status)| {
            json!({
                "game_id": found.game_id,
                "store": found.store,
                "store_id": found.store_id,
                "path": found.path,
                "status": status.as_str(),
            })
        })
        .collect();
    json!({
        "installations": installations,
        "refused": detection.refused,
        "skipped": detection.skipped,
    })
}

fn print_human(detection: &Detection, statuses: &[Status]) {
    if detection.confirmed.is_empty() {
        println!("No installed games found in the Steam, GOG or Epic libraries.");
    }
    for (found, status) in detection.confirmed.iter().zip(statuses) {
        let note = match status {
            Status::New => "new; run 'modn game detect --register' to register it",
            Status::Registered => "registered",
            Status::AlreadyRegistered => "already registered",
        };
        println!(
            "{}: {} ({} {}) - {note}",
            found.game_id,
            found.path.display(),
            found.store.as_str(),
            found.store_id
        );
    }
    for refused in &detection.refused {
        println!(
            "Refused {}: {} ({} {}): {}",
            refused.game_id,
            refused.path.display(),
            refused.store.as_str(),
            refused.store_id,
            refused.reason
        );
    }
    for skipped in &detection.skipped {
        println!("Skipped {}: {}", skipped.game_id, skipped.reason);
    }
}
