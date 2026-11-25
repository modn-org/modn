mod layout;
mod listing;
mod record;
mod recovery;
mod removal;
#[cfg(test)]
#[path = "registry/tests.rs"]
mod tests;

use self::record::RegistrationRecord;
use super::css::CssChecker;
use super::digest::theme_digest;
use super::error::{Result, ThemeError};
use super::inspect::{inspect_theme_dir, InspectedTheme};
use super::limits::is_valid_id;
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
    #[cfg(test)]
    failpoint: std::sync::atomic::AtomicU8,
}

impl ThemeRegistry {
    /// Use the theme storage of the given Modn data directory.
    #[must_use]
    pub fn new(data_directory: &Path) -> Self {
        Self {
            root: storage::root(data_directory),
            #[cfg(test)]
            failpoint: std::sync::atomic::AtomicU8::new(0),
        }
    }

    #[cfg(test)]
    pub(in crate::themes::registry) fn fail_at(&self, point: layout::Failpoint) {
        self.failpoint
            .store(point.code(), std::sync::atomic::Ordering::SeqCst);
    }

    fn next_failpoint(&self) -> Option<layout::Failpoint> {
        #[cfg(test)]
        {
            let code = self.failpoint.swap(0, std::sync::atomic::Ordering::SeqCst);
            layout::Failpoint::from_code(code)
        }
        #[cfg(not(test))]
        {
            None
        }
    }

    /// Inspect, copy and register one local theme directory.
    ///
    /// The source is first copied into a bounded private inspection directory
    /// outside the registry. Only that copy is inspected. After inspection
    /// passes, the inspected copy is copied into a staging directory inside the
    /// registry, hashed again and installed, so the bytes that passed
    /// inspection are the bytes that registration stores. A refused
    /// registration therefore creates no registry state. Registering an
    /// existing `(game, id)` replaces the earlier copy and keeps a recoverable
    /// backup until the replacement is complete. The source directory is never
    /// modified.
    pub fn register(&self, source: &Path, checker: &dyn CssChecker) -> Result<RegisteredTheme> {
        let inspection = tempfile::Builder::new()
            .prefix("modn-theme-inspect-")
            .tempdir()
            .map_err(|error| ThemeError::io("cannot create a theme inspection directory", error))?;
        let inspected_copy = layout::theme_dir(inspection.path());
        copy_tree(source, &inspected_copy)?;
        let inspected = inspect_theme_dir(&inspected_copy, checker).map_err(ThemeError::Invalid)?;
        let game = inspected.manifest.game.clone();
        let id = inspected.manifest.id.clone();

        let _lock = storage::lock(&self.root)?;
        let registry = layout::registry_root(&self.root);
        fs::create_dir_all(&registry)
            .map_err(|error| ThemeError::io("cannot create the theme registry", error))?;
        storage::ensure_real_directory(&registry, "theme registry")?;
        let _recovery_problems = recovery::recover(&self.root)?;
        let staging = tempfile::Builder::new()
            .prefix(layout::STAGING_PREFIX)
            .tempdir_in(&registry)
            .map_err(|error| ThemeError::io("cannot create a theme staging directory", error))?;
        let copy = layout::theme_dir(staging.path());
        copy_tree(&inspected_copy, &copy)?;
        let digest = theme_digest(&copy)
            .map_err(|problem| ThemeError::Invalid(vec![with_label(problem, &game, &id)]))?;
        if digest != inspected.digest {
            return Err(ThemeError::corrupt(
                &copy,
                "the inspected theme changed while it was being copied",
            ));
        }
        let record = RegistrationRecord {
            schema_version: layout::SCHEMA_VERSION,
            game: game.clone(),
            id: id.clone(),
            blake3: digest.clone(),
            registered_at: unix_seconds(),
        };
        let target = layout::entry_root(&registry, &game, &id);
        let backup = layout::backup_root(&registry, &game, &id);
        layout::install(
            staging.path(),
            &registry,
            &target,
            &backup,
            &record,
            self.next_failpoint(),
        )?;
        Ok(RegisteredTheme {
            game,
            id,
            name: inspected.manifest.name,
            variants: inspected.manifest.variants,
            icon: inspected.manifest.icon,
            blake3: digest,
            registered_at: record.registered_at,
            dir: layout::theme_dir(&target),
        })
    }

    /// List every valid registration and every skipped one.
    ///
    /// Interrupted replacements are recovered first. A recovery problem is a
    /// diagnostic, never a failed list.
    pub fn list(&self) -> Result<ListedThemes> {
        match fs::symlink_metadata(&self.root) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(ListedThemes {
                    themes: Vec::new(),
                    problems: Vec::new(),
                });
            }
            Err(error) => {
                return Err(ThemeError::io(
                    "cannot inspect the theme data directory",
                    error,
                ));
            }
            Ok(_) => {}
        }
        let _lock = storage::lock(&self.root)?;
        let mut problems = recovery::recover(&self.root)?;
        let mut listed = listing::list(&self.root)?;
        problems.append(&mut listed.problems);
        listed.problems = problems;
        Ok(listed)
    }

    /// Load one registered theme after verifying its stored digest.
    pub fn load(&self, game: &str, id: &str, checker: &dyn CssChecker) -> Result<InspectedTheme> {
        check_id(game)?;
        check_id(id)?;
        match fs::symlink_metadata(&self.root) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(ThemeError::NotRegistered { id: id.to_owned() });
            }
            Err(error) => {
                return Err(ThemeError::io(
                    "cannot inspect the theme data directory",
                    error,
                ));
            }
            Ok(_) => {}
        }
        let _lock = storage::lock(&self.root)?;
        let _recovery_problems = recovery::recover(&self.root)?;
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
        removal::remove(&self.root, id, game)
    }
}

struct CopyBudget {
    bytes: u64,
    entries: usize,
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)
        .map_err(|error| ThemeError::io("cannot create the copied theme directory", error))?;
    copy_entries(
        source,
        destination,
        &mut CopyBudget {
            bytes: 0,
            entries: 0,
        },
    )
}

fn copy_entries(source: &Path, destination: &Path, budget: &mut CopyBudget) -> Result<()> {
    let entries = fs::read_dir(source)
        .map_err(|error| ThemeError::io("cannot read the theme source directory", error))?;
    for entry in entries {
        let entry = entry
            .map_err(|error| ThemeError::io("cannot read the theme source directory", error))?;
        budget.entries = budget.entries.saturating_add(1);
        if budget.entries > layout::MAX_COPY_ENTRIES {
            return Err(too_large(format!(
                "the theme has more than {} entries",
                layout::MAX_COPY_ENTRIES
            )));
        }
        let file_type = entry
            .file_type()
            .map_err(|error| ThemeError::io("cannot inspect a theme source entry", error))?;
        let from = entry.path();
        let to = destination.join(entry.file_name());
        if file_type.is_dir() {
            fs::create_dir(&to)
                .map_err(|error| ThemeError::io("cannot create a copied theme directory", error))?;
            copy_entries(&from, &to, budget)?;
        } else if file_type.is_file() {
            let length = entry
                .metadata()
                .map_err(|error| ThemeError::io("cannot inspect a theme source file", error))?
                .len();
            if budget.bytes.saturating_add(length) > layout::MAX_COPY_BYTES {
                return Err(too_large(format!(
                    "the theme is larger than {} bytes",
                    layout::MAX_COPY_BYTES
                )));
            }
            let copied = fs::copy(&from, &to).map_err(|error| {
                ThemeError::io(format!("cannot copy \"{}\"", from.display()), error)
            })?;
            budget.bytes = budget.bytes.saturating_add(copied);
            if budget.bytes > layout::MAX_COPY_BYTES {
                return Err(too_large(format!(
                    "the theme is larger than {} bytes",
                    layout::MAX_COPY_BYTES
                )));
            }
        } else {
            return Err(ThemeError::corrupt(
                &from,
                "the theme source changed while it was being copied; links and special files are refused",
            ));
        }
    }
    Ok(())
}

fn too_large(message: String) -> ThemeError {
    ThemeError::Invalid(vec![
        ThemeProblem::new(Rule::ThemeTooLarge, message).with_path(".")
    ])
}

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

fn manifest_problem(game: &str, id: &str, message: impl Into<String>) -> ThemeProblem {
    ThemeProblem::new(Rule::ManifestInvalid, message)
        .with_path(format!(
            "{}/{game}/{id}/{}",
            layout::REGISTRY,
            layout::RECORD
        ))
        .with_theme(format!("{game}/{id}"))
}

fn with_label(mut problem: ThemeProblem, game: &str, id: &str) -> ThemeProblem {
    if problem.theme.is_none() {
        problem.theme = Some(format!("{game}/{id}"));
    }
    problem
}

fn check_id(value: &str) -> Result<()> {
    if is_valid_id(value) {
        Ok(())
    } else {
        Err(ThemeError::InvalidId(value.to_owned()))
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}
