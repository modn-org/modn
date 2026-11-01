//! Catalog provenance and dependency links recorded with managed packages.
//!
//! Callers resolve bucket manifests; this module only sees verified archive
//! digests, so readiness and uninstall protection stay game- and bucket-agnostic.
use super::validation::validate_transaction_root;
use crate::integrity::package_intact;
use crate::state::Store;
use anyhow::{bail, Result};
use rusqlite::{params, Transaction};
use serde::Serialize;
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

/// A catalog release to install, optionally as one member of a stack.
#[derive(Clone, Copy)]
pub(super) struct CatalogInstall<'a> {
    pub(super) release: &'a CatalogRelease,
    pub(super) stack_id: Option<&'a str>,
}

/// Provenance and dependency links written in the same commit as ownership.
pub(super) struct CatalogRecord<'a> {
    pub(super) release: &'a CatalogRelease,
    pub(super) dependency_ids: Vec<String>,
    /// Uncommitted stack that rolls this package back if the stack fails.
    pub(super) stack_id: Option<&'a str>,
}

/// Installed state of one catalog dependency in one game installation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
#[non_exhaustive]
pub enum DependencyState {
    /// Managed packages with the pinned digest whose content is intact.
    Ready {
        /// Managed package IDs that satisfy the dependency.
        package_ids: Vec<String>,
    },
    /// Managed packages with the pinned digest whose content has drifted.
    Damaged {
        /// Managed package IDs to restore or uninstall.
        package_ids: Vec<String>,
    },
    /// Managed packages from the same catalog mod with a different digest.
    OtherRelease {
        /// Managed package IDs installed from another release of the mod.
        package_ids: Vec<String>,
    },
    /// No managed package matches the dependency.
    Missing,
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
        let mut satisfied = Vec::new();
        for (dependency, state) in dependencies
            .iter()
            .zip(self.dependency_states(game, dependencies)?)
        {
            let DependencyState::Ready { package_ids } = state else {
                bail!(
                    "dependency not installed or intact: {}; install or restore it first",
                    dependency.reference
                );
            };
            satisfied.extend(package_ids);
        }
        satisfied.sort();
        satisfied.dedup();
        Ok(satisfied)
    }

    /// Report the installed state of each dependency, in the given order.
    ///
    /// Uses the same digest and integrity rules as [`Store::ready_dependencies`].
    /// Another release is recognized only through recorded catalog provenance.
    /// This read never locks or writes and does not authorize installation.
    pub fn dependency_states(
        &self,
        game: &Path,
        dependencies: &[CatalogDependency],
    ) -> Result<Vec<DependencyState>> {
        if dependencies.is_empty() {
            return Ok(Vec::new());
        }
        let game = validate_transaction_root(game)?;
        let installed = self.installed(&game)?;
        dependencies
            .iter()
            .map(|dependency| {
                let (intact, damaged): (Vec<_>, Vec<_>) = installed
                    .iter()
                    .filter(|package| package.archive_sha256 == dependency.archive_sha256)
                    .partition(|package| package_intact(self, &game, package));
                if !intact.is_empty() {
                    return Ok(DependencyState::Ready {
                        package_ids: intact.iter().map(|p| p.id.clone()).collect(),
                    });
                }
                if !damaged.is_empty() {
                    return Ok(DependencyState::Damaged {
                        package_ids: damaged.iter().map(|p| p.id.clone()).collect(),
                    });
                }
                let recorded = provenance_packages(self, &dependency.reference)?;
                let package_ids: Vec<String> = installed
                    .iter()
                    .filter(|package| recorded.contains(&package.id))
                    .map(|package| package.id.clone())
                    .collect();
                Ok(if package_ids.is_empty() {
                    DependencyState::Missing
                } else {
                    DependencyState::OtherRelease { package_ids }
                })
            })
            .collect()
    }
}

fn provenance_packages(store: &Store, reference: &CatalogReference) -> Result<Vec<String>> {
    // A database from before catalog provenance, opened read-only, is not
    // migrated. It has no table and therefore records no provenance.
    let recorded: bool = store.connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master
         WHERE type='table' AND name='catalog_provenance')",
        [],
        |row| row.get(0),
    )?;
    if !recorded {
        return Ok(Vec::new());
    }
    let mut statement = store.connection.prepare(
        "SELECT package_id FROM catalog_provenance
         WHERE bucket=?1 AND game_id=?2 AND mod_id=?3 ORDER BY package_id",
    )?;
    let rows = statement.query_map(
        params![reference.bucket, reference.game_id, reference.mod_id],
        |row| row.get(0),
    )?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
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
    if let Some(stack_id) = record.stack_id {
        tx.execute(
            "INSERT INTO pending_stack_packages(stack_id,ord,package_id)
             SELECT ?1, COUNT(*), ?2 FROM pending_stack_packages WHERE stack_id=?1",
            params![stack_id, package_id],
        )?;
    }
    Ok(())
}

pub(super) fn forget(tx: &Transaction<'_>, package_id: &str) -> Result<()> {
    tx.execute(
        "DELETE FROM pending_stack_packages WHERE package_id=?1",
        [package_id],
    )?;
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
