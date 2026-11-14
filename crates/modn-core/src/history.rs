//! Durable history of committed install, update and uninstall operations.
//!
//! A record is written in the same SQLite transaction as the ownership
//! change it describes. It captures every package release and owned file the
//! operation touched, with the before and after content digests, so a later
//! undo needs no other state.

use crate::buckets::PackageSource;
use crate::state::Store;
use crate::transactions::{CatalogReference, ReleaseDeclarations};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

mod persist;
mod release;
pub(crate) use persist::{
    insert, install_operation, package_snapshot, read, stack_operation, uninstall_operation,
    uninstall_operation_many,
};
pub(crate) use release::{
    ensure_schema as ensure_release_schema, metadata_for, package_metadata, record_metadata,
};

/// What a committed operation did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum HistoryKind {
    /// A new package was installed.
    Install,
    /// An installed package was replaced by another release.
    Update,
    /// An installed package was removed.
    Uninstall,
}

impl HistoryKind {
    /// The kind as it appears in JSON and the history table.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Update => "update",
            Self::Uninstall => "uninstall",
        }
    }

    fn from_label(label: &str) -> Option<Self> {
        match label {
            "install" => Some(Self::Install),
            "update" => Some(Self::Update),
            "uninstall" => Some(Self::Uninstall),
            _ => None,
        }
    }
}

/// A standalone archive installed without a catalog release.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct StandalonePackage {
    /// User-facing mod name, normally the archive stem.
    pub mod_id: String,
    /// Version when the caller knows one.
    pub version: Option<String>,
    /// Where the archive came from; a local archive is a manual source.
    pub source: PackageSource,
}

impl StandalonePackage {
    /// Describe one archive without a catalog release.
    #[must_use]
    pub fn new(mod_id: impl Into<String>, source: PackageSource) -> Self {
        Self {
            mod_id: mod_id.into(),
            version: None,
            source,
        }
    }
}

/// Per-operation input that the caller sets before a managed write.
///
/// Every committed install, update and uninstall made by the CLI uses one.
/// Library callers that set none write no history.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct HistoryContext {
    /// Registered game ID.
    pub game_id: String,
    /// Canonical installation root.
    pub installation: PathBuf,
    /// Release source keyed by the qualified `bucket/game/mod` reference.
    pub sources: BTreeMap<String, PackageSource>,
    /// Source used by a package without a catalog reference.
    pub standalone: Option<StandalonePackage>,
    /// Operation that this write undoes.
    pub undoes: Option<String>,
}

impl HistoryContext {
    /// A context for one operation on `installation`.
    #[must_use]
    pub fn new(game_id: impl Into<String>, installation: impl Into<PathBuf>) -> Self {
        Self {
            game_id: game_id.into(),
            installation: installation.into(),
            sources: BTreeMap::new(),
            standalone: None,
            undoes: None,
        }
    }

    /// Record that the next write undoes `operation_id`.
    #[must_use]
    pub fn with_undo_of(mut self, operation_id: impl Into<String>) -> Self {
        self.undoes = Some(operation_id.into());
        self
    }

    /// Record `source` for the catalog reference of `reference`.
    #[must_use]
    pub fn with_source(mut self, reference: &CatalogReference, source: PackageSource) -> Self {
        self.sources.insert(reference.to_string(), source);
        self
    }

    /// Record the descriptor of a standalone archive.
    #[must_use]
    pub fn with_standalone(mut self, package: StandalonePackage) -> Self {
        self.standalone = Some(package);
        self
    }
}

/// One release side of a history record.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct HistoryRelease {
    /// Release version, when the source states one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Qualified catalog identity, when the release came from a bucket.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog: Option<CatalogReference>,
    /// Declared dependencies, conflicts and native-injector status.
    ///
    /// `None` in a record written before this data was stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declarations: Option<ReleaseDeclarations>,
    /// Descriptor that can download the archive again.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<PackageSource>,
    /// Archive BLAKE3 pin (hex), when recorded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive_blake3: Option<String>,
}

/// One owned file touched by an operation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct HistoryFile {
    /// Installation root label: `game` or `content`.
    pub root: String,
    /// Root-relative destination path.
    pub destination: String,
    /// Content digest before the operation, when the file existed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest_before: Option<String>,
    /// Content digest after the operation, when the file remains.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest_after: Option<String>,
}

/// One package released or removed by an operation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct HistoryPackage {
    /// Catalog mod ID, or the standalone archive stem.
    pub mod_id: String,
    /// Managed-package ID before the operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_id_before: Option<String>,
    /// Managed-package ID after the operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_id_after: Option<String>,
    /// Release that the operation replaced or removed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_before: Option<HistoryRelease>,
    /// Release that the operation installed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_after: Option<HistoryRelease>,
    /// Owned files the operation created, replaced or removed.
    pub files: Vec<HistoryFile>,
}

/// One committed manager operation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct HistoryOperation {
    /// Stable, short operation ID.
    pub id: String,
    /// What the operation did.
    pub kind: HistoryKind,
    /// Registered game ID.
    pub game_id: String,
    /// Canonical installation root.
    pub installation: PathBuf,
    /// UTC completion time in ISO 8601 form.
    pub recorded_at: String,
    /// Packages the operation changed.
    pub packages: Vec<HistoryPackage>,
    /// Operation this record undoes, when the command was `modn undo`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undoes: Option<String>,
}

/// Release metadata persisted for a managed package.
#[derive(Clone, Debug)]
pub(crate) struct ReleaseMetadata {
    pub mod_id: String,
    pub version: Option<String>,
    pub source: Option<PackageSource>,
    pub archive_blake3: Option<String>,
    pub catalog: Option<CatalogReference>,
    pub declarations: Option<ReleaseDeclarations>,
}

/// One file digest for a new or removed release.
pub(crate) struct FileSpec {
    pub root: String,
    pub destination: String,
    pub digest: String,
}

/// Compute the `blake3:<hex>` digest of `path`.
pub(crate) fn digest_blake3(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    io::copy(&mut file, &mut hasher)?;
    Ok(format!("blake3:{}", hasher.finalize().to_hex()))
}

impl Store {
    /// Record later managed installs, updates and uninstalls in history.
    pub fn set_history_context(&mut self, context: HistoryContext) {
        self.history = Some(context);
    }

    /// Committed operations for the manager state, newest first.
    ///
    /// `game_id` limits the result to one registered game.
    pub fn history(&self, game_id: Option<&str>) -> Result<Vec<HistoryOperation>> {
        read(&self.connection, game_id)
    }

    /// The history record that the most recent write committed, if any.
    ///
    /// The committing transaction records the ID, so a concurrent write on
    /// another [`Store`] cannot change what this write reports. A write that
    /// commits no record reports none. A write that commits several records
    /// reports the newest one, which `modn undo` without an ID reverses first.
    #[must_use]
    pub fn committed_operation_id(&self) -> Option<&str> {
        self.committed_operation.as_deref()
    }
}
