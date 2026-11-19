//! The local theme registry: validated copies, digests and removal.
//!
//! Each registration is a byte-for-byte copy of an inspected theme directory
//! plus one `registration.json`. The registry lives under the Modn data
//! directory, never in the inspected source directory.

mod layout;
mod record;
#[cfg(test)]
#[path = "registry/tests.rs"]
mod tests;

use self::record::RegistrationRecord;
use super::css::CssChecker;
use super::digest::theme_digest;
use super::error::{Result, ThemeError};
use super::inspect::{inspect_theme_dir, InspectedTheme};
use super::limits::{is_valid_id, MAX_THEME_MANIFEST_BYTES};
use super::manifest::{ThemeManifest, MANIFEST_FILE};
use super::problem::{Rule, ThemeProblem};
use super::storage;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// One registered theme, as stored in the local theme registry.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredTheme {
    game: String,
    id: String,
    name: String,
    variants: Vec<String>,
    icon: Option<String>,
    blake3: String,
    registered_at: u64,
    dir: PathBuf,
}

impl RegisteredTheme {
    /// Game id the theme belongs to.
    #[must_use]
    pub fn game(&self) -> &str {
        &self.game
    }

    /// Theme id.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Display name from the copied `theme.json`.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Variant ids from the copied `theme.json`.
    #[must_use]
    pub fn variants(&self) -> &[String] {
        &self.variants
    }

    /// Icon path from the copied `theme.json`, relative to the theme directory.
    #[must_use]
    pub fn icon(&self) -> Option<&str> {
        self.icon.as_deref()
    }

    /// BLAKE3 digest recorded when the theme was registered.
    #[must_use]
    pub fn blake3(&self) -> &str {
        &self.blake3
    }

    /// Unix seconds when the theme was registered.
    #[must_use]
    pub fn registered_at(&self) -> u64 {
        self.registered_at
    }

    /// Directory of the byte-for-byte theme copy.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

/// Every valid registration together with the problems of the skipped ones.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedThemes {
    /// Valid registrations, sorted by game then id.
    pub themes: Vec<RegisteredTheme>,
    /// Problems for entries that were skipped. One bad entry never fails the list.
    pub problems: Vec<ThemeProblem>,
}

/// The registration that a removal deleted.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovedTheme {
    game: String,
    id: String,
}

impl RemovedTheme {
    /// Game id of the removed registration.
    #[must_use]
    pub fn game(&self) -> &str {
        &self.game
    }

    /// Theme id of the removed registration.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// Storage for local theme registrations under one Modn data directory.
#[derive(Debug)]
pub struct ThemeRegistry {
    root: PathBuf,
}

impl ThemeRegistry {
    /// Use the theme storage of the given Modn data directory.
    #[must_use]
    pub fn new(data_directory: &Path) -> Self {
        Self {
            root: storage::root(data_directory),
        }
    }

    /// Inspect, copy and register one local theme directory.
    ///
    /// Registration validates the source first. A valid source is copied into a
    /// staging directory, hashed again and moved into place. Registering an
    /// existing `(game, id)` replaces the earlier copy. The source directory is
    /// never modified.
    pub fn register(&self, source: &Path, checker: &dyn CssChecker) -> Result<RegisteredTheme> {
        let inspected = inspect_theme_dir(source, checker).map_err(ThemeError::Invalid)?;
        let game = inspected.manifest.game.clone();
        let id = inspected.manifest.id.clone();
        let registered_at = unix_seconds();
        let _lock = storage::lock(&self.root)?;
        let registry = layout::registry_root(&self.root);
        fs::create_dir_all(&registry)
            .map_err(|error| ThemeError::io("cannot create the theme registry", error))?;
        storage::ensure_real_directory(&registry, "theme registry")?;
        layout::remove_stale_staging(&registry)?;
        let staging = tempfile::Builder::new()
            .prefix(layout::STAGING_PREFIX)
            .tempdir_in(&registry)
            .map_err(|error| ThemeError::io("cannot create a theme staging directory", error))?;
        let copy = layout::theme_dir(staging.path());
        copy_tree(source, &copy)?;
        let digest = theme_digest(&copy)
            .map_err(|problem| ThemeError::Invalid(vec![with_label(problem, &game, &id)]))?;
        if digest != inspected.digest {
            return Err(ThemeError::corrupt(
                &copy,
                "the theme source changed while it was being copied",
            ));
        }
        let record = RegistrationRecord {
            schema_version: layout::SCHEMA_VERSION,
            game: game.clone(),
            id: id.clone(),
            blake3: digest.clone(),
            registered_at,
        };
        record::write(staging.path(), &record)?;
        let target = layout::entry_root(&registry, &game, &id);
        layout::install(staging.path(), &registry, &target)?;
        Ok(RegisteredTheme {
            game,
            id,
            name: inspected.manifest.name,
            variants: inspected.manifest.variants,
            icon: inspected.manifest.icon,
            blake3: digest,
            registered_at,
            dir: layout::theme_dir(&target),
        })
    }

    /// List every valid registration and every skipped one.
    pub fn list(&self) -> Result<ListedThemes> {
        let registry = layout::registry_root(&self.root);
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
            let game =
                game.map_err(|error| ThemeError::io("cannot list the theme registry", error))?;
            let name = game.file_name();
            let Some(name) = name.to_str().map(str::to_owned) else {
                continue;
            };
            if name.starts_with(layout::STAGING_PREFIX) {
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

    /// Load one registered theme after verifying its stored digest.
    pub fn load(&self, game: &str, id: &str, checker: &dyn CssChecker) -> Result<InspectedTheme> {
        check_id(game)?;
        check_id(id)?;
        let registry = layout::registry_root(&self.root);
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
        let record = record::read(&entry)
            .map_err(|reason| ThemeError::corrupt(entry.join(layout::RECORD), reason))?;
        if record.game != game || record.id != id {
            return Err(ThemeError::corrupt(
                entry.join(layout::RECORD),
                "registration.json names a different game or id",
            ));
        }
        let dir = layout::theme_dir(&entry);
        let digest = theme_digest(&dir)
            .map_err(|problem| ThemeError::Invalid(vec![with_label(problem, game, id)]))?;
        if digest != record.blake3 {
            return Err(ThemeError::Invalid(vec![digest_mismatch(
                game,
                id,
                &record.blake3,
                &digest,
            )]));
        }
        inspect_theme_dir(&dir, checker).map_err(ThemeError::Invalid)
    }

    /// Remove one registration.
    ///
    /// With no game, an id that is registered for exactly one game is removed.
    /// An id registered for several games is ambiguous and left untouched.
    pub fn remove(&self, id: &str, game: Option<&str>) -> Result<RemovedTheme> {
        check_id(id)?;
        if let Some(game) = game {
            check_id(game)?;
        }
        let registry = layout::registry_root(&self.root);
        let _lock = storage::lock(&self.root)?;
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
                fs::remove_dir_all(&entry).map_err(|error| {
                    ThemeError::io("cannot remove the theme registration", error)
                })?;
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

/// Read one registration and turn every defect into a diagnostic.
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

/// Read the copied `theme.json` for the display fields of a listing.
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

/// Copy every regular file below `source` into `destination`.
fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)
        .map_err(|error| ThemeError::io("cannot create the copied theme directory", error))?;
    copy_entries(source, destination)
}

/// Copy one directory level and recurse into real subdirectories.
fn copy_entries(source: &Path, destination: &Path) -> Result<()> {
    let entries = fs::read_dir(source)
        .map_err(|error| ThemeError::io("cannot read the theme source directory", error))?;
    for entry in entries {
        let entry = entry
            .map_err(|error| ThemeError::io("cannot read the theme source directory", error))?;
        let file_type = entry
            .file_type()
            .map_err(|error| ThemeError::io("cannot inspect a theme source entry", error))?;
        let from = entry.path();
        let to = destination.join(entry.file_name());
        if file_type.is_dir() {
            fs::create_dir(&to)
                .map_err(|error| ThemeError::io("cannot create a copied theme directory", error))?;
            copy_entries(&from, &to)?;
        } else if file_type.is_file() {
            fs::copy(&from, &to).map_err(|error| {
                ThemeError::io(format!("cannot copy \"{}\"", from.display()), error)
            })?;
        } else {
            return Err(ThemeError::corrupt(
                &from,
                "the theme source changed while it was being copied; links and special files are refused",
            ));
        }
    }
    Ok(())
}

/// Report a digest that does not match the recorded one.
fn digest_mismatch(game: &str, id: &str, expected: &str, actual: &str) -> ThemeProblem {
    ThemeProblem::new(
        Rule::DigestMismatch,
        format!("stored digest {expected} does not match the copied theme digest {actual}"),
    )
    .with_path(format!(
        "{}/{game}/{id}/{}",
        layout::REGISTRY,
        layout::THEME
    ))
    .with_theme(format!("{game}/{id}"))
}

/// Report a registration record that cannot be used.
fn manifest_problem(game: &str, id: &str, message: impl Into<String>) -> ThemeProblem {
    ThemeProblem::new(Rule::ManifestInvalid, message)
        .with_path(format!(
            "{}/{game}/{id}/{}",
            layout::REGISTRY,
            layout::RECORD
        ))
        .with_theme(format!("{game}/{id}"))
}

/// Report a registry directory that does not have a usable name.
fn invalid_name_problem(path: &str, message: impl Into<String>) -> ThemeProblem {
    ThemeProblem::new(Rule::ManifestInvalid, message).with_path(path.to_owned())
}

/// Attach the `<game>/<id>` label when the source problem has none.
fn with_label(mut problem: ThemeProblem, game: &str, id: &str) -> ThemeProblem {
    if problem.theme.is_none() {
        problem.theme = Some(format!("{game}/{id}"));
    }
    problem
}

/// Reject an id before it reaches a path.
fn check_id(value: &str) -> Result<()> {
    if is_valid_id(value) {
        Ok(())
    } else {
        Err(ThemeError::InvalidId(value.to_owned()))
    }
}

/// Current Unix time in whole seconds.
fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[cfg(test)]
thread_local! {
    static INJECT_SWAP_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Make the next replacement of an existing registration fail after the old
/// copy moved aside, so the restore path can be exercised.
#[cfg(test)]
fn fail_next_swap() {
    INJECT_SWAP_FAILURE.with(|flag| flag.set(true));
}

#[cfg(test)]
fn take_injected_swap_failure() -> bool {
    INJECT_SWAP_FAILURE.with(std::cell::Cell::get)
}
