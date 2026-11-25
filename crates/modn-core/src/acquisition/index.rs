use super::entry_name::parse_entry_name;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub(super) const INDEX_NAME: &str = ".digest-index.json";

const INDEX_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ArchivePin {
    hex: String,
}

impl ArchivePin {
    pub(super) fn blake3(digest: &str) -> Option<Self> {
        (digest.len() == 64 && is_lower_hex(digest)).then(|| Self {
            hex: digest.to_owned(),
        })
    }

    fn encode(&self) -> String {
        format!("blake3:{}", self.hex)
    }

    fn decode(value: &str) -> Option<Self> {
        value.strip_prefix("blake3:").and_then(Self::blake3)
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct IndexFile {
    version: u32,
    entries: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ArchiveIndex {
    entries: BTreeMap<ArchivePin, String>,
}

impl ArchiveIndex {
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

    pub(super) fn file_name(&self, pin: &ArchivePin) -> Option<&str> {
        self.entries.get(pin).map(String::as_str)
    }

    pub(super) fn insert(&mut self, pin: ArchivePin, name: String) {
        self.entries.insert(pin, name);
    }

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

    pub(super) fn record(root: &Path, pin: ArchivePin, name: String) {
        let mut index = Self::read(root).unwrap_or_default();
        index.insert(pin, name);
        let _ = index.write(root);
    }
}

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
