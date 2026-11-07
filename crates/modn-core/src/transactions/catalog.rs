use super::NativeInjectorPolicy;
use crate::integrity::package_intact;
use crate::state::Store;
use anyhow::{bail, Result};
use rusqlite::{params, OptionalExtension, Transaction};
use serde::Serialize;
use std::fmt;
use std::path::Path;

/// Qualified catalog identity of one mod: `bucket/game/mod`.
#[expect(missing_docs, reason = "Field and variant names state their meaning")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct CatalogReference {
    pub bucket: String,
    pub game_id: String,
    pub mod_id: String,
}

#[expect(missing_docs, reason = "Method names state their behavior")]
impl CatalogReference {
    #[must_use]
    pub fn new(bucket: String, game_id: String, mod_id: String) -> Self {
        Self {
            bucket,
            game_id,
            mod_id,
        }
    }
}

/// A catalog mod that must not share a game installation with a release.
#[expect(missing_docs, reason = "Field and variant names state their meaning")]
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CatalogConflict {
    pub reference: CatalogReference,
    /// Archive digest the bucket pins for that mod, when it publishes one.
    /// A managed package with this digest counts as that mod.
    pub archive_sha256: Option<String>,
}

#[expect(missing_docs, reason = "Method names state their behavior")]
impl CatalogConflict {
    #[must_use]
    pub fn new(reference: CatalogReference, archive_sha256: Option<String>) -> Self {
        Self {
            reference,
            archive_sha256,
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
            requirement: None,
        }
    }

    /// Record the manifest's version requirement, such as `>=1.31.0`, so a
    /// later update of the dependency can check it.
    #[must_use]
    pub fn with_requirement(mut self, requirement: Option<String>) -> Self {
        self.requirement = requirement;
        self
    }
}

/// A catalog dependency that a managed package recorded at installation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RecordedDependency {
    /// The catalog mod that the dependent's manifest named.
    pub reference: CatalogReference,
    /// The archive digest that the dependency's release pinned at installation.
    pub archive_sha256: String,
    /// The manifest's version requirement, if any.
    pub requirement: Option<String>,
}

#[expect(missing_docs, reason = "Method names state their behavior")]
impl CatalogRelease {
    #[must_use]
    pub fn new(reference: CatalogReference, dependencies: Vec<CatalogDependency>) -> Self {
        Self {
            reference,
            dependencies,
            conflicts: Vec::new(),
            version: None,
            native_injector: false,
        }
    }

    /// Mark whether this release is a native injector.
    #[must_use]
    pub fn with_native_injector(mut self, native_injector: bool) -> Self {
        self.native_injector = native_injector;
        self
    }

    #[must_use]
    pub fn with_conflicts(mut self, conflicts: Vec<CatalogConflict>) -> Self {
        self.conflicts = conflicts;
        self
    }

    /// Supply the target version for dependent requirement checks during update.
    #[must_use]
    pub fn with_version(mut self, version: String) -> Self {
        self.version = Some(version);
        self
    }

    /// Refuse this release under `policy` when it is a native injector.
    pub fn ensure_native_injector_allowed(&self, policy: NativeInjectorPolicy) -> Result<()> {
        policy.check(&self.reference, self.native_injector)
    }
}

impl fmt::Display for CatalogReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}/{}", self.bucket, self.game_id, self.mod_id)
    }
}

/// One required catalog release, identified by its manifest-pinned archive digest.
#[expect(missing_docs, reason = "Field and variant names state their meaning")]
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CatalogDependency {
    /// Catalog identity used in diagnostics.
    pub reference: CatalogReference,
    pub archive_sha256: String,
    /// The dependent manifest's version requirement for this dependency.
    pub requirement: Option<String>,
}

/// Catalog identity and prerequisites for one managed install.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CatalogRelease {
    /// Target version, when known to the catalog caller.
    pub version: Option<String>,
    /// Catalog identity recorded as the managed package's provenance.
    pub reference: CatalogReference,
    /// Releases that must already be managed and intact in the same game installation.
    pub dependencies: Vec<CatalogDependency>,
    /// Catalog mods that must not be managed in the same game installation.
    pub conflicts: Vec<CatalogConflict>,
    /// True when the release places a native library that the game process loads.
    pub native_injector: bool,
}

#[derive(Clone, Copy)]
pub(super) struct CatalogInstall<'a> {
    pub(super) release: &'a CatalogRelease,
    pub(super) stack_id: Option<&'a str>,
    pub(super) replacing: Option<super::update::Replacement<'a>>,
}

pub(super) struct CatalogRecord<'a> {
    pub(super) release: &'a CatalogRelease,
    pub(super) dependency_ids: Vec<String>,
    pub(super) stack_id: Option<&'a str>,
    pub(super) replacing: Option<super::update::Replacement<'a>>,
}

/// Installed state of one catalog dependency in one game installation.
#[expect(missing_docs, reason = "Field and variant names state their meaning")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
#[non_exhaustive]
pub enum DependencyState {
    /// Managed packages with the pinned digest whose content is intact.
    Ready { package_ids: Vec<String> },
    /// Managed packages with the pinned digest whose content has drifted.
    Damaged { package_ids: Vec<String> },
    /// Managed packages from the same catalog mod with a different digest.
    OtherRelease { package_ids: Vec<String> },
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
        let game = self.transaction_root(game)?;
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

impl Store {
    /// The catalog dependencies that managed package `package_id` recorded
    /// when it was installed, or `None` if it predates these records or has
    /// no catalog dependencies.
    pub fn recorded_dependencies(
        &self,
        package_id: &str,
    ) -> Result<Option<Vec<RecordedDependency>>> {
        if !self.has_table("package_requirements")? {
            return Ok(None);
        }
        let mut statement = self.connection.prepare(
            "SELECT bucket, game_id, mod_id, archive_sha256, requirement
             FROM package_requirements WHERE package_id=?1 ORDER BY bucket, game_id, mod_id",
        )?;
        let recorded = statement
            .query_map([package_id], |row| {
                Ok(RecordedDependency {
                    reference: CatalogReference {
                        bucket: row.get(0)?,
                        game_id: row.get(1)?,
                        mod_id: row.get(2)?,
                    },
                    archive_sha256: row.get(3)?,
                    requirement: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((!recorded.is_empty()).then_some(recorded))
    }
}

fn provenance_packages(store: &Store, reference: &CatalogReference) -> Result<Vec<String>> {
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
    for dependency in &record.release.dependencies {
        let reference = &dependency.reference;
        tx.execute(
            "INSERT OR REPLACE INTO package_requirements
                 (package_id,bucket,game_id,mod_id,archive_sha256,requirement)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                package_id,
                reference.bucket,
                reference.game_id,
                reference.mod_id,
                dependency.archive_sha256,
                dependency.requirement
            ],
        )?;
    }
    if let Some(stack_id) = record.stack_id {
        tx.execute(
            "INSERT INTO pending_stack_packages(stack_id,ord,package_id)
             SELECT ?1, COUNT(*), ?2 FROM pending_stack_packages WHERE stack_id=?1",
            params![stack_id, package_id],
        )?;
    }
    super::conflicts::record(tx, package_id, record.release)?;
    Ok(())
}

pub(super) fn forget(tx: &Transaction<'_>, package_id: &str) -> Result<()> {
    super::conflicts::forget(tx, package_id)?;
    tx.execute(
        "DELETE FROM package_requirements WHERE package_id=?1",
        [package_id],
    )?;
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

pub(super) fn reference_of(store: &Store, package_id: &str) -> Result<Option<CatalogReference>> {
    if !store.has_table("catalog_provenance")? {
        return Ok(None);
    }
    Ok(store
        .connection
        .query_row(
            "SELECT bucket, game_id, mod_id FROM catalog_provenance WHERE package_id=?1",
            [package_id],
            |row| {
                Ok(CatalogReference {
                    bucket: row.get(0)?,
                    game_id: row.get(1)?,
                    mod_id: row.get(2)?,
                })
            },
        )
        .optional()?)
}

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
