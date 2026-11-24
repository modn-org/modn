//! One persisted theme selection per game.

#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;

use super::error::{Result, ThemeError};
use super::limits::is_valid_id;
use super::storage;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Directory under the theme root that holds one file per game.
const SELECTION: &str = "selection";
/// Current shape of one selection file.
const SCHEMA_VERSION: u32 = 1;
/// Largest accepted selection file, far above its real size.
const MAX_SELECTION_BYTES: u64 = 4096;
/// Suffix of every selection file.
const SUFFIX: &str = ".json";

/// The stored choice for one game.
///
/// A selection is either a theme choice or the explicit none choice, which
/// keeps the default theme even when the game has an official theme.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    game: String,
    theme: Option<String>,
    variant: Option<String>,
}

impl Selection {
    /// Game the choice applies to.
    #[must_use]
    pub fn game(&self) -> &str {
        &self.game
    }

    /// Selected theme id, or the empty string for the none choice.
    ///
    /// Prefer [`Selection::theme_id`], which distinguishes the none choice from
    /// a stored theme id.
    #[must_use]
    pub fn theme(&self) -> &str {
        self.theme_id().unwrap_or_default()
    }

    /// Selected theme id, or `None` for the none choice.
    #[must_use]
    pub fn theme_id(&self) -> Option<&str> {
        self.theme.as_deref()
    }

    /// Whether this selection asks for the default theme.
    #[must_use]
    pub fn is_none(&self) -> bool {
        self.theme.is_none()
    }

    /// Selected variant id, or `None` for the base stylesheet.
    #[must_use]
    pub fn variant(&self) -> Option<&str> {
        self.variant.as_deref()
    }
}

/// Persisted selection of at most one theme per game.
#[derive(Debug)]
pub struct SelectionStore {
    root: PathBuf,
}

impl SelectionStore {
    /// Use the theme storage of the given Modn data directory.
    #[must_use]
    pub fn new(data_directory: &Path) -> Self {
        Self {
            root: storage::root(data_directory),
        }
    }

    /// Store `theme` as the choice for `game`.
    ///
    /// The selection does not check that the theme is registered. Resolution
    /// does that later and falls back when the theme is absent.
    pub fn select(&self, game: &str, theme: &str, variant: Option<&str>) -> Result<Selection> {
        check_id(game)?;
        check_id(theme)?;
        if let Some(variant) = variant {
            check_id(variant)?;
        }
        let selection = Selection {
            game: game.to_owned(),
            theme: Some(theme.to_owned()),
            variant: variant.map(str::to_owned),
        };
        self.store(&selection)?;
        Ok(selection)
    }

    /// Store the explicit default-theme choice for `game`.
    ///
    /// The choice replaces any earlier selection and survives later resolves,
    /// so the official theme for the game does not apply until a `select` call
    /// replaces it.
    pub fn select_none(&self, game: &str) -> Result<Selection> {
        check_id(game)?;
        let selection = Selection {
            game: game.to_owned(),
            theme: None,
            variant: None,
        };
        self.store(&selection)?;
        Ok(selection)
    }

    /// Write `selection` as the choice for its game, replacing any earlier one.
    fn store(&self, selection: &Selection) -> Result<()> {
        let _lock = storage::lock(&self.root)?;
        let directory = self.directory();
        fs::create_dir_all(&directory).map_err(|error| {
            ThemeError::io("cannot create the theme selection directory", error)
        })?;
        storage::ensure_real_directory(&directory, "theme selection directory")?;
        let record = SelectionRecord::from_selection(selection);
        let bytes = serde_json::to_vec(&record).map_err(|error| {
            ThemeError::corrupt(
                self.path(&selection.game),
                format!("cannot encode the theme selection: {error}"),
            )
        })?;
        let mut temporary = tempfile::NamedTempFile::new_in(&directory)
            .map_err(|error| ThemeError::io("cannot create a theme selection file", error))?;
        temporary
            .write_all(&bytes)
            .map_err(|error| ThemeError::io("cannot write the theme selection", error))?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|error| ThemeError::io("cannot sync the theme selection", error))?;
        temporary
            .persist(self.path(&selection.game))
            .map_err(|error| ThemeError::io("cannot store the theme selection", error.error))?;
        Ok(())
    }

    /// Read the choice for `game`, or `None` when no file exists.
    ///
    /// An unreadable or invalid file is [`ThemeError::Corrupt`], never `None`.
    pub fn get(&self, game: &str) -> Result<Option<Selection>> {
        check_id(game)?;
        read_selection(&self.path(game), game)
    }

    /// Delete the choice for `game` and report whether a file existed.
    pub fn clear(&self, game: &str) -> Result<bool> {
        check_id(game)?;
        let _lock = storage::lock(&self.root)?;
        match fs::remove_file(self.path(game)) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(ThemeError::io("cannot remove the theme selection", error)),
        }
    }

    /// Delete every selection file and return the games that were cleared.
    ///
    /// The returned game ids are sorted and unique.
    pub fn clear_all(&self) -> Result<Vec<String>> {
        let _lock = storage::lock(&self.root)?;
        let directory = self.directory();
        let entries = match fs::read_dir(&directory) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Vec::new());
            }
            Err(error) => {
                return Err(ThemeError::io(
                    "cannot list the theme selection directory",
                    error,
                ));
            }
            Ok(entries) => entries,
        };
        let mut games = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                ThemeError::io("cannot list the theme selection directory", error)
            })?;
            if !entry
                .file_type()
                .map_err(|error| ThemeError::io("cannot inspect a theme selection", error))?
                .is_file()
            {
                continue;
            }
            let Some(game) = selection_stem(&entry.file_name()) else {
                continue;
            };
            if !is_valid_id(&game) {
                continue;
            }
            match fs::remove_file(entry.path()) {
                Ok(()) => games.push(game),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(ThemeError::io("cannot remove the theme selection", error));
                }
            }
        }
        games.sort();
        Ok(games)
    }

    /// Read every stored selection, sorted by game.
    ///
    /// A corrupt file is [`ThemeError::Corrupt`] and is not skipped.
    pub fn list(&self) -> Result<Vec<Selection>> {
        let directory = self.directory();
        let entries = match fs::read_dir(&directory) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Vec::new());
            }
            Err(error) => {
                return Err(ThemeError::io(
                    "cannot list the theme selection directory",
                    error,
                ));
            }
            Ok(entries) => entries,
        };
        let mut selections = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                ThemeError::io("cannot list the theme selection directory", error)
            })?;
            if !entry
                .file_type()
                .map_err(|error| ThemeError::io("cannot inspect a theme selection", error))?
                .is_file()
            {
                continue;
            }
            let Some(game) = selection_stem(&entry.file_name()) else {
                continue;
            };
            if !is_valid_id(&game) {
                continue;
            }
            if let Some(selection) = read_selection(&entry.path(), &game)? {
                selections.push(selection);
            }
        }
        selections.sort_by(|left, right| left.game.cmp(&right.game));
        Ok(selections)
    }

    /// Directory holding every selection file.
    fn directory(&self) -> PathBuf {
        self.root.join(SELECTION)
    }

    /// File holding the selection for one game.
    fn path(&self, game: &str) -> PathBuf {
        self.directory().join(format!("{game}{SUFFIX}"))
    }
}

/// The `theme` field of one record: a theme id, or `null` for the none choice.
///
/// A missing field is an error, so a truncated record is never read as none.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum StoredTheme {
    /// A theme choice.
    Id(String),
    /// The explicit none choice, stored as `null`.
    None,
}

/// Serialized shape of one selection file.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectionRecord {
    schema_version: u32,
    game: String,
    theme: StoredTheme,
    variant: Option<String>,
}

impl SelectionRecord {
    /// Build the record that stores `selection`.
    fn from_selection(selection: &Selection) -> Self {
        let theme = match &selection.theme {
            Some(id) => StoredTheme::Id(id.clone()),
            None => StoredTheme::None,
        };
        Self {
            schema_version: SCHEMA_VERSION,
            game: selection.game.clone(),
            theme,
            variant: selection.variant.clone(),
        }
    }

    /// Rebuild the selection stored in this record.
    fn into_selection(self) -> Selection {
        let theme = match self.theme {
            StoredTheme::Id(id) => Some(id),
            StoredTheme::None => None,
        };
        Selection {
            game: self.game,
            theme,
            variant: self.variant,
        }
    }

    /// Check every field, including that the record belongs to `game`.
    fn validate(&self, game: &str) -> std::result::Result<(), String> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(format!(
                "selection schema_version {} is not {SCHEMA_VERSION}",
                self.schema_version
            ));
        }
        if self.game != game {
            return Err(format!(
                "selection file for \"{game}\" names game \"{}\"",
                self.game
            ));
        }
        if !is_valid_id(&self.game) {
            return Err(format!(
                "selection game \"{}\" is not a valid game id",
                self.game
            ));
        }
        match &self.theme {
            StoredTheme::Id(theme) => {
                if !is_valid_id(theme) {
                    return Err(format!(
                        "selection theme \"{theme}\" is not a valid theme id"
                    ));
                }
            }
            StoredTheme::None => {
                if self.variant.is_some() {
                    return Err("a none selection must not name a variant".to_owned());
                }
            }
        }
        if let Some(variant) = &self.variant {
            if !is_valid_id(variant) {
                return Err(format!(
                    "selection variant \"{variant}\" is not a valid variant id"
                ));
            }
        }
        Ok(())
    }
}

/// Read and validate the selection file for `game`.
fn read_selection(path: &Path, game: &str) -> Result<Option<Selection>> {
    let metadata = match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ThemeError::io("cannot inspect the theme selection", error)),
        Ok(metadata) => metadata,
    };
    if !metadata.file_type().is_file() {
        return Err(ThemeError::corrupt(
            path,
            "a theme selection must be a regular file",
        ));
    }
    if metadata.len() > MAX_SELECTION_BYTES {
        return Err(ThemeError::corrupt(
            path,
            format!("the theme selection exceeds the {MAX_SELECTION_BYTES}-byte limit"),
        ));
    }
    let bytes =
        fs::read(path).map_err(|error| ThemeError::io("cannot read the theme selection", error))?;
    let record: SelectionRecord = serde_json::from_slice(&bytes).map_err(|error| {
        ThemeError::corrupt(path, format!("the theme selection is not valid: {error}"))
    })?;
    record
        .validate(game)
        .map_err(|reason| ThemeError::corrupt(path, reason))?;
    Ok(Some(record.into_selection()))
}

/// Return the game id of a `<game>.json` file name.
fn selection_stem(name: &OsStr) -> Option<String> {
    name.to_str()
        .and_then(|name| name.strip_suffix(SUFFIX))
        .map(str::to_owned)
}

/// Reject an id before it reaches a path.
fn check_id(value: &str) -> Result<()> {
    if is_valid_id(value) {
        Ok(())
    } else {
        Err(ThemeError::InvalidId(value.to_owned()))
    }
}
