//! Digest index for the verified archive cache.
//!
//! The index maps an [`ArchivePin`] to the cache entry file name that holds
//! those bytes. It is an optimisation only: a hit is always re-verified
//! against the file before it is returned, and a missing, unreadable or
//! unknown-version index is rebuilt from a scan. Modn is pre-alpha, so an
//! index in an unrecognised format is corrupt by definition and is never
//! migrated.
use super::parse_entry_name;
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

/// The digest that pins cached archive bytes.
///
/// A pin is one hash algorithm plus its lowercase hex digest. The cache index
/// is keyed by this type, so changing the algorithm is a local change. Issue
/// #117 switches pins to BLAKE3 with no index migration: the old index parses
/// as an unknown algorithm and is rebuilt from a scan.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ArchivePin {
    algorithm: PinAlgorithm,
    hex: String,
}

/// Hash algorithms an [`ArchivePin`] can name. SHA-256 is the only one today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PinAlgorithm {
    Sha256,
}

impl ArchivePin {
    /// Parse a lowercase SHA-256 pin, or `None` when the digest is malformed.
    pub(super) fn sha256(digest: &str) -> Option<Self> {
        (digest.len() == PinAlgorithm::Sha256.hex_len() && is_lower_hex(digest)).then(|| Self {
            algorithm: PinAlgorithm::Sha256,
            hex: digest.to_owned(),
        })
    }

    /// The persisted `algorithm:hex` form.
    fn encode(&self) -> String {
        format!("{}:{}", self.algorithm.tag(), self.hex)
    }

    fn decode(value: &str) -> Option<Self> {
        let (tag, hex) = value.split_once(':')?;
        let algorithm = PinAlgorithm::from_tag(tag)?;
        (hex.len() == algorithm.hex_len() && is_lower_hex(hex)).then(|| Self {
            algorithm,
            hex: hex.to_owned(),
        })
    }
}

impl PinAlgorithm {
    fn tag(self) -> &'static str {
        match self {
            Self::Sha256 => "sha256",
        }
    }

    fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "sha256" => Some(Self::Sha256),
            _ => None,
        }
    }

    fn hex_len(self) -> usize {
        match self {
            Self::Sha256 => 64,
        }
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

    /// Whether the index records no entry.
    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Cache entry file name recorded for `pin`, if any.
    pub(super) fn file_name(&self, pin: &ArchivePin) -> Option<&str> {
        self.entries.get(pin).map(String::as_str)
    }

    /// Record `name` for `pin`, replacing any earlier name.
    pub(super) fn insert(&mut self, pin: ArchivePin, name: String) {
        self.entries.insert(pin, name);
    }

    /// Record `name` for `pin` only when the pin has no entry yet, so a scan
    /// keeps the first entry in listing order as the recorded hit.
    pub(super) fn insert_first(&mut self, pin: ArchivePin, name: String) {
        self.entries.entry(pin).or_insert(name);
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
    /// A missing or corrupt index becomes an index with this entry only; the
    /// next lookup that needs another pin rebuilds the rest from a scan. Index
    /// write failures are ignored because the index is never authority.
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
