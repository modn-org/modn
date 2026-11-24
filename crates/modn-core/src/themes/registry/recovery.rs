//! Recovery of interrupted theme registrations.
//!
//! A replacement moves the previous registration to `.backup/<game>/<id>`
//! before the new copy moves in. If the process stops, the next operation
//! restores that backup or, when the live copy verifies, deletes it.

use super::super::digest::theme_digest;
use super::super::error::{Result, ThemeError};
use super::super::limits::is_valid_id;
use super::super::problem::{Rule, ThemeProblem};
use super::{layout, record};
use std::fs;
use std::io;
use std::path::Path;

/// Restore or clean interrupted replacements below one theme root.
///
/// Backups are handled before staging directories are removed. A per-entry
/// problem becomes a diagnostic; only a failure to list the registry is an
/// error.
pub(super) fn recover(root: &Path) -> Result<Vec<ThemeProblem>> {
    let registry = layout::registry_root(root);
    let mut problems = Vec::new();
    recover_backups(&registry, &mut problems)?;
    problems.extend(layout::remove_stale_staging(&registry)?);
    Ok(problems)
}

/// Restore every backup whose live registration is missing and delete every
/// backup whose live registration still verifies.
fn recover_backups(registry: &Path, problems: &mut Vec<ThemeProblem>) -> Result<()> {
    let backups = registry.join(layout::BACKUP);
    let games = match fs::read_dir(&backups) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(ThemeError::io(
                "cannot list the theme registry backup directory",
                error,
            ));
        }
        Ok(games) => games,
    };
    for game in games {
        let game = game.map_err(|error| {
            ThemeError::io("cannot list the theme registry backup directory", error)
        })?;
        let Some(name) = game.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let path = format!("{}/{}/{name}", layout::REGISTRY, layout::BACKUP);
        let Ok(file_type) = game.file_type() else {
            problems.push(backup_problem(&path, "cannot inspect a theme backup entry"));
            continue;
        };
        if !file_type.is_dir() {
            problems.push(backup_problem(
                &path,
                "a theme backup entry must be a directory",
            ));
            continue;
        }
        if !is_valid_id(&name) {
            problems.push(backup_problem(
                &path,
                format!("backup directory name \"{name}\" is not a valid game id"),
            ));
            continue;
        }
        let entries = match fs::read_dir(game.path()) {
            Ok(entries) => entries,
            Err(error) => {
                problems.push(backup_problem(
                    &path,
                    format!("cannot list the theme backups: {error}"),
                ));
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    problems.push(backup_problem(
                        &path,
                        format!("cannot list the theme backups: {error}"),
                    ));
                    break;
                }
            };
            let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let label = format!("{path}/{id}");
            let Ok(file_type) = entry.file_type() else {
                problems.push(backup_problem(&label, "cannot inspect a theme backup"));
                continue;
            };
            if !file_type.is_dir() {
                problems.push(backup_problem(&label, "a theme backup must be a directory"));
                continue;
            }
            if !is_valid_id(&id) {
                problems.push(backup_problem(
                    &label,
                    format!("backup directory name \"{id}\" is not a valid theme id"),
                ));
                continue;
            }
            recover_one(registry, &name, &id, &entry.path(), &label, problems);
        }
    }
    Ok(())
}

/// Restore or clean one backup.
fn recover_one(
    registry: &Path,
    game: &str,
    id: &str,
    backup: &Path,
    label: &str,
    problems: &mut Vec<ThemeProblem>,
) {
    let live = layout::entry_root(registry, game, id);
    match fs::symlink_metadata(&live) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let Some(parent) = live.parent() else {
                problems.push(backup_problem(
                    label,
                    "the theme registration has no parent directory",
                ));
                return;
            };
            if let Err(error) = fs::create_dir_all(parent) {
                problems.push(backup_problem(
                    label,
                    format!("cannot restore the previous theme registration: {error}"),
                ));
                return;
            }
            if let Err(error) = fs::rename(backup, &live) {
                problems.push(backup_problem(
                    label,
                    format!("cannot restore the previous theme registration: {error}"),
                ));
            }
        }
        Ok(metadata) if metadata.file_type().is_dir() => match verify(&live, game, id) {
            Ok(()) => {
                if let Err(error) = fs::remove_dir_all(backup) {
                    problems.push(backup_problem(
                        label,
                        format!("cannot delete the verified backup: {error}"),
                    ));
                }
            }
            Err(reason) => problems.push(backup_problem(
                label,
                format!(
                    "the live registration fails verification ({reason}); the previous copy is kept"
                ),
            )),
        },
        Ok(_) => problems.push(backup_problem(
            label,
            "the live theme registration is not a directory; the previous copy is kept",
        )),
        Err(error) => problems.push(backup_problem(
            label,
            format!("cannot inspect the live theme registration: {error}"),
        )),
    }
}

/// Check the record and digest of one live registration.
fn verify(live: &Path, game: &str, id: &str) -> std::result::Result<(), String> {
    let record = record::read(live)?;
    if record.game != game || record.id != id {
        return Err("registration.json names a different game or id".to_owned());
    }
    let digest = theme_digest(&layout::theme_dir(live)).map_err(|problem| problem.to_string())?;
    if digest != record.blake3 {
        return Err("the copied theme no longer matches its digest".to_owned());
    }
    Ok(())
}

/// Report one recovery problem without failing the operation.
fn backup_problem(path: &str, message: impl Into<String>) -> ThemeProblem {
    ThemeProblem::new(Rule::ManifestInvalid, message).with_path(path.to_owned())
}
