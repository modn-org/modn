//! Read the game folders that the Steam, GOG and Epic libraries list.
//!
//! This code knows store formats, not games. A game module declares its store
//! identities as data; [`crate::detection`] matches them against the
//! installations read here.

mod epic;
mod gog;
mod platform;
mod steam;
#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_LIBRARY_FILE_BYTES: u64 = 1024 * 1024;
const MAX_LIBRARY_ENTRIES: usize = 4096;

/// Game store that installs games into a library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Storefront {
    /// Valve Steam.
    Steam,
    /// GOG Galaxy.
    Gog,
    /// Epic Games Launcher.
    Epic,
}

impl Storefront {
    /// Lowercase name used in manager state, JSON and `module.json`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Steam => "steam",
            Self::Gog => "gog",
            Self::Epic => "epic",
        }
    }

    /// Store with the lowercase name `label`, if any.
    #[must_use]
    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "steam" => Some(Self::Steam),
            "gog" => Some(Self::Gog),
            "epic" => Some(Self::Epic),
            _ => None,
        }
    }
}

/// One game folder that a store library lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct StoreInstallation {
    /// Store whose library lists the folder.
    pub store: Storefront,
    /// Store identity of the game, such as a Steam app ID.
    pub store_id: String,
    /// Game folder as the library records it; not yet confirmed.
    pub path: PathBuf,
}

/// One GOG Galaxy installation record, as the Windows registry lists it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct GogRecord {
    /// GOG product ID (the registry `gameID` value).
    pub product_id: String,
    /// Game folder (the registry `path` value).
    pub path: PathBuf,
}

/// Library locations to read.
///
/// [`StoreLibraries::system`] reads the locations of this computer. Tests and
/// embedding callers supply fixture locations instead.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct StoreLibraries {
    /// Steam folders. Each holds `steamapps/libraryfolders.vdf`.
    #[serde(default)]
    pub steam_roots: Vec<PathBuf>,
    /// Folders that hold Epic Games Launcher `.item` manifests.
    #[serde(default)]
    pub epic_manifest_dirs: Vec<PathBuf>,
    /// GOG Galaxy installation records.
    #[serde(default)]
    pub gog_records: Vec<GogRecord>,
}

impl StoreLibraries {
    /// Library locations from explicit roots.
    #[must_use]
    pub fn new(
        steam_roots: Vec<PathBuf>,
        epic_manifest_dirs: Vec<PathBuf>,
        gog_records: Vec<GogRecord>,
    ) -> Self {
        Self {
            steam_roots,
            epic_manifest_dirs,
            gog_records,
        }
    }

    /// Library locations of this computer: the Windows registry, the Steam
    /// folder and the Epic manifest folder. Empty on other platforms.
    ///
    /// Test builds never read real libraries. They read the JSON file that
    /// `MODN_STORE_LIBRARIES` names, or use no libraries.
    #[must_use]
    pub fn system() -> Self {
        test_override().unwrap_or_else(platform::libraries)
    }

    /// Every game folder that these libraries list.
    ///
    /// Unreadable, oversized or malformed library entries are skipped, because
    /// one broken store record must not hide the others.
    #[must_use]
    pub fn installations(&self) -> Vec<StoreInstallation> {
        let mut installations = Vec::new();
        for root in &self.steam_roots {
            installations.extend(steam::installations(root));
        }
        for directory in &self.epic_manifest_dirs {
            installations.extend(epic::installations(directory));
        }
        installations.extend(gog::installations(&self.gog_records));
        installations
    }
}

#[cfg(any(test, feature = "test-instruction-budget"))]
fn test_override() -> Option<StoreLibraries> {
    let libraries = std::env::var_os("MODN_STORE_LIBRARIES")
        .and_then(|path| fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    Some(libraries)
}

#[cfg(not(any(test, feature = "test-instruction-budget")))]
fn test_override() -> Option<StoreLibraries> {
    None
}

fn read_bounded(path: &Path) -> Option<String> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_LIBRARY_FILE_BYTES {
        return None;
    }
    let mut text = String::new();
    File::open(path)
        .ok()?
        .take(MAX_LIBRARY_FILE_BYTES)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
}

fn directory_files(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    entries
        .flatten()
        .take(MAX_LIBRARY_ENTRIES)
        .map(|entry| entry.path())
        .collect()
}

/// Whether `value` has the shape of a `store` identity.
///
/// Steam app IDs and GOG product IDs are decimal. Epic app names are short
/// ASCII names.
pub(crate) fn valid_store_id(store: Storefront, value: &str) -> bool {
    let digits = |max_len: usize| {
        !value.is_empty() && value.len() <= max_len && value.bytes().all(|b| b.is_ascii_digit())
    };
    match store {
        Storefront::Steam => digits(10),
        Storefront::Gog => digits(20),
        Storefront::Epic => {
            !value.is_empty()
                && value.len() <= 64
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        }
    }
}
