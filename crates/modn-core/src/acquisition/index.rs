//! Digest index for the verified archive cache.
//!
//! The index maps an [`ArchivePin`] to the cache entry file name that holds
//! those bytes. It is an optimisation only: a hit is always re-verified
//! against the file before it is returned, and a missing, unreadable or
//! unknown-format index is replaced by a scan that is kept in memory. A lookup
//! never writes the index; only storing a downloaded entry persists it. Modn
//! is pre-alpha, so an index in an unrecognised format is corrupt by
//! definition and is never migrated. That includes an index that still names
//! `sha256` pins.
use super::entry_name::parse_entry_name;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Index file name inside the cache root. [`parse_entry_name`] never accepts
/// it, so it is not a cache entry.
pub(super) const INDEX_NAME: &str = ".digest-index.json";

const INDEX_VERSION: u32 = 1;

/// The BLAKE3 digest that pins cached archive bytes.
///
/// A pin is the `blake3:` tag plus 64 lowercase hex characters. The cache
/// index is keyed by this type. An index written for SHA-256 pins holds
/// `sha256:` keys, which decode to nothing, so the whole index is rebuilt
/// from a scan and its entries are re-verified before use.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ArchivePin {
    hex: String,
}

impl ArchivePin {
    /// Parse a lowercase BLAKE3 pin, or `None` when the digest is malformed.
    pub(super) fn blake3(digest: &str) -> Option<Self> {
        (digest.len() == 64 && is_lower_hex(digest)).then(|| Self {
            hex: digest.to_owned(),
        })
    }

    /// The persisted `blake3:hex` form.
    fn encode(&self) -> String {
        format!("blake3:{}", self.hex)
    }

    fn decode(value: &str) -> Option<Self> {
        value.strip_prefix("blake3:").and_then(Self::blake3)
    }
}

/// Persisted index content. `deny_unknown_fields` and the version check make
/// any future format fail to parse, which triggers a scan and rebuild.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct IndexFile {
    version: u32,
    entries: BTreeMap<String, String>,
}

/// Pin-to-file-name map loaded from, or written to, the cache root.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ArchiveIndex {
    entries: BTreeMap<ArchivePin, String>,
}

impl ArchiveIndex {
    /// Read the index. `None` means missing, corrupt or an unknown format.
    pub(super) fn read(root: &Path) -> Option<Self> {
        let bytes = fs::read(index_path(root)).ok()?;
        let file: IndexFile = serde_json::from_slice(&bytes).ok()?;
        if file.version != INDEX_VERSION {
            return None;
        }
        let mut entries = BTreeMap::new();
        for (pin, name) in file.entries {
            parse_entry_name(&name)?;
            entries.insert(ArchivePin::decode(&pin)?, name);
        }
        Some(Self { entries })
    }

    /// Cache entry file name recorded for `pin`, if any.
    pub(super) fn file_name(&self, pin: &ArchivePin) -> Option<&str> {
        self.entries.get(pin).map(String::as_str)
    }

    /// Record `name` for `pin`, replacing any earlier name.
    pub(super) fn insert(&mut self, pin: ArchivePin, name: String) {
        self.entries.insert(pin, name);
    }

    /// Write the index atomically: a temporary file in the cache root, then a
    /// rename. A crash therefore leaves the old index or no index, both of
    /// which a later lookup handles.
    pub(super) fn write(&self, root: &Path) -> Result<()> {
        fs::create_dir_all(root)?;
        let file = IndexFile {
            version: INDEX_VERSION,
            entries: self
                .entries
                .iter()
                .map(|(pin, name)| (pin.encode(), name.clone()))
                .collect(),
        };
        let mut temp = tempfile::Builder::new()
            .prefix(".digest-index-")
            .suffix(".tmp")
            .tempfile_in(root)?;
        serde_json::to_writer(temp.as_file_mut(), &file)?;
        temp.as_file_mut().write_all(b"\n")?;
        temp.as_file().sync_all()?;
        temp.persist(index_path(root))?;
        Ok(())
    }

    /// Record one newly stored entry, keeping entries the current index holds.
    ///
    /// A missing or corrupt index becomes an index with this entry only; a
    /// later lookup that needs another pin scans the cache for it in memory.
    /// Index write failures are ignored because the index is never authority.
    pub(super) fn record(root: &Path, pin: ArchivePin, name: String) {
        let mut index = Self::read(root).unwrap_or_default();
        index.insert(pin, name);
        let _ = index.write(root);
    }
}

/// Path of the index file for cache `root`.
pub(super) fn index_path(root: &Path) -> PathBuf {
    root.join(INDEX_NAME)
}

fn is_lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests;
