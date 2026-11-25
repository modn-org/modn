use super::super::digest::theme_digest;
use super::super::error::{Result, ThemeError};
use super::super::limits::{is_valid_id, MAX_THEME_MANIFEST_BYTES};
use super::super::manifest::{ThemeManifest, MANIFEST_FILE};
use super::super::problem::{Rule, ThemeProblem};
use super::layout;
use super::record;
use super::{digest_mismatch, manifest_problem, with_label, ListedThemes, RegisteredTheme};
use std::fs;
use std::io;
use std::path::Path;

pub(super) fn list(root: &Path) -> Result<ListedThemes> {
    let registry = layout::registry_root(root);
    let mut themes = Vec::new();
    let mut problems = Vec::new();
    let games = match fs::read_dir(&registry) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(ListedThemes { themes, problems });
        }
        Err(error) => return Err(ThemeError::io("cannot list the theme registry", error)),
        Ok(games) => games,
    };
    for game in games {
        let game = game.map_err(|error| ThemeError::io("cannot list the theme registry", error))?;
        let name = game.file_name();
        let Some(name) = name.to_str().map(str::to_owned) else {
            continue;
        };
        if name.starts_with(layout::STAGING_PREFIX) || name == layout::BACKUP {
            continue;
        }
        let path = format!("{}/{name}", layout::REGISTRY);
        let Ok(file_type) = game.file_type() else {
            problems.push(invalid_name_problem(
                &path,
                "cannot inspect a theme registry entry",
            ));
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        if !is_valid_id(&name) {
            problems.push(invalid_name_problem(
                &path,
                format!("registry directory name \"{name}\" is not a valid game id"),
            ));
            continue;
        }
        let entries = match fs::read_dir(game.path()) {
            Ok(entries) => entries,
            Err(error) => {
                problems.push(invalid_name_problem(
                    &path,
                    format!("cannot list the game registrations: {error}"),
                ));
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    problems.push(invalid_name_problem(
                        &path,
                        format!("cannot list the game registrations: {error}"),
                    ));
                    break;
                }
            };
            let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Ok(file_type) = entry.file_type() else {
                problems.push(invalid_name_problem(
                    &format!("{path}/{id}"),
                    "cannot inspect a theme registration",
                ));
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }
            if !is_valid_id(&id) {
                problems.push(invalid_name_problem(
                    &format!("{path}/{id}"),
                    format!("registry directory name \"{id}\" is not a valid theme id"),
                ));
                continue;
            }
            match read_entry(&entry.path(), &name, &id) {
                Ok(theme) => themes.push(theme),
                Err(problem) => problems.push(problem),
            }
        }
    }
    themes.sort_by(|left, right| {
        left.game
            .cmp(&right.game)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(ListedThemes { themes, problems })
}

fn read_entry(
    entry: &Path,
    game: &str,
    id: &str,
) -> std::result::Result<RegisteredTheme, ThemeProblem> {
    let record = record::read(entry).map_err(|reason| manifest_problem(game, id, reason))?;
    if record.game != game || record.id != id {
        return Err(manifest_problem(
            game,
            id,
            "registration.json names a different game or id",
        ));
    }
    let dir = layout::theme_dir(entry);
    let digest = theme_digest(&dir).map_err(|problem| with_label(problem, game, id))?;
    if digest != record.blake3 {
        return Err(digest_mismatch(game, id, &record.blake3, &digest));
    }
    let manifest = read_copy_manifest(&dir).map_err(|reason| manifest_problem(game, id, reason))?;
    if manifest.game != game || manifest.id != id {
        return Err(manifest_problem(
            game,
            id,
            "the copied manifest names a different game or id",
        ));
    }
    Ok(RegisteredTheme {
        game: game.to_owned(),
        id: id.to_owned(),
        name: manifest.name,
        variants: manifest.variants,
        icon: manifest.icon,
        blake3: record.blake3,
        registered_at: record.registered_at,
        dir,
    })
}

fn read_copy_manifest(dir: &Path) -> std::result::Result<ThemeManifest, String> {
    let path = dir.join(MANIFEST_FILE);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("cannot inspect theme.json: {error}"))?;
    if !metadata.file_type().is_file() {
        return Err("theme.json is not a regular file".to_owned());
    }
    if metadata.len() > MAX_THEME_MANIFEST_BYTES {
        return Err(format!(
            "theme.json exceeds the {MAX_THEME_MANIFEST_BYTES}-byte limit"
        ));
    }
    let bytes = fs::read(&path).map_err(|error| format!("cannot read theme.json: {error}"))?;
    ThemeManifest::parse(&bytes).map_err(|problem| problem.to_string())
}

fn invalid_name_problem(path: &str, message: impl Into<String>) -> ThemeProblem {
    ThemeProblem::new(Rule::ManifestInvalid, message).with_path(path.to_owned())
}
