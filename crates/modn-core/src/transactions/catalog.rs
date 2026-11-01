//! Catalog provenance and dependency links recorded with managed packages.
//!
//! Callers resolve bucket manifests; this module only sees verified archive
//! digests, so readiness and uninstall protection stay game- and bucket-agnostic.
use super::validation::validate_transaction_root;
use crate::integrity::package_intact;
use crate::state::Store;
use anyhow::{bail, Result};
use rusqlite::{params, Transaction};
use std::fmt;
use std::path::Path;

/// Qualified catalog identity of one mod: `bucket/game/mod`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CatalogReference {
    /// Bucket that published the package manifest.
    pub bucket: String,
    /// Game identifier the package manifest targets.
    pub game_id: String,
    /// Package identifier within the bucket and game.
    pub mod_id: String,
}

impl CatalogReference {
    /// Identify one package manifest by bucket, game and package ID.
    #[must_use]
    pub fn new(bucket: String, game_id: String, mod_id: String) -> Self {
        Self {
            bucket,
            game_id,
            mod_id,
        }
    }
}

impl CatalogDependency {
    /// Require the release whose archive digest is pinned by its manifest.
    #[must_use]
    pub fn new(reference: CatalogReference, archive_sha256: String) -> Self {
        Self {
            reference,
            archive_sha256,
        }
    }
}

impl CatalogRelease {
    /// Describe one catalog release and the releases it requires.
    #[must_use]
    pub fn new(reference: CatalogReference, dependencies: Vec<CatalogDependency>) -> Self {
        Self {
            reference,
            dependencies,
        }
    }
}

impl fmt::Display for CatalogReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}/{}", self.bucket, self.game_id, self.mod_id)
    }
}

/// One required catalog release, identified by its manifest-pinned archive digest.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CatalogDependency {
    /// Catalog identity used in diagnostics.
    pub reference: CatalogReference,
    /// SHA-256 digest pinned by the dependency's package manifest.
    pub archive_sha256: String,
}

/// Catalog identity and prerequisites for one managed install.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CatalogRelease {
    /// Catalog identity recorded as the managed package's provenance.
    pub reference: CatalogReference,
    /// Releases that must already be managed and intact in the same game installation.
    pub dependencies: Vec<CatalogDependency>,
}

/// Provenance and dependency links written in the same commit as ownership.
pub(super) struct CatalogRecord<'a> {
    pub(super) release: &'a CatalogRelease,
    pub(super) dependency_ids: Vec<String>,
}

impl Store {
    /// Return the managed packages that satisfy every dependency.
    ///
    /// A dependency is satisfied only by a managed package with the pinned
    /// archive digest whose owned files and original payload bytes are intact.
    /// A ledger entry alone is not proof that a prerequisite is still present.
    /// The check reads only the ledger and the filesystem, never game discovery.
    /// This read does not lock; installation repeats it under the transaction lock.
    pub fn ready_dependencies(
        &self,
        game: &Path,
        dependencies: &[CatalogDependency],
    ) -> Result<Vec<String>> {
        if dependencies.is_empty() {
            return Ok(Vec::new());
        }
        let game = validate_transaction_root(game)?;
        let installed = self.installed(&game)?;
        let mut satisfied = Vec::new();
        for dependency in dependencies {
            let before = satisfied.len();
            satisfied.extend(
                installed
                    .iter()
                    .filter(|package| package.archive_sha256 == dependency.archive_sha256)
                    .filter(|package| package_intact(self, &game, package))
                    .map(|package| package.id.clone()),
            );
            if satisfied.len() == before {
                bail!(
                    "dependency not installed or intact: {}; install or restore it first",
                    dependency.reference
                );
            }
        }
        satisfied.sort();
        satisfied.dedup();
        Ok(satisfied)
    }
}

pub(super) fn record(tx: &Transaction<'_>, package_id: &str, record: &CatalogRecord) -> Result<()> {
    let reference = &record.release.reference;
    tx.execute(
        "INSERT INTO catalog_provenance(package_id,bucket,game_id,mod_id) VALUES (?1,?2,?3,?4)",
        params![
            package_id,
            reference.bucket,
            reference.game_id,
            reference.mod_id
        ],
    )?;
    for dependency in &record.dependency_ids {
        tx.execute(
            "INSERT INTO package_dependencies(package_id,dependency_id) VALUES (?1,?2)",
            params![package_id, dependency],
        )?;
    }
    Ok(())
}

pub(super) fn forget(tx: &Transaction<'_>, package_id: &str) -> Result<()> {
    tx.execute(
        "DELETE FROM package_dependencies WHERE package_id=?1",
        [package_id],
    )?;
    tx.execute(
        "DELETE FROM catalog_provenance WHERE package_id=?1",
        [package_id],
    )?;
    Ok(())
}

/// Managed packages that recorded `package_id` as a dependency, with their catalog identity.
pub(super) fn dependents(store: &Store, package_id: &str) -> Result<Vec<String>> {
    let mut statement = store.connection.prepare(
        "SELECT d.package_id, c.bucket, c.game_id, c.mod_id
         FROM package_dependencies d
         JOIN catalog_provenance c ON c.package_id = d.package_id
         WHERE d.dependency_id=?1 ORDER BY d.package_id",
    )?;
    let rows = statement.query_map([package_id], |row| {
        let reference = CatalogReference {
            bucket: row.get(1)?,
            game_id: row.get(2)?,
            mod_id: row.get(3)?,
        };
        Ok(format!("{} ({reference})", row.get::<_, String>(0)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}
