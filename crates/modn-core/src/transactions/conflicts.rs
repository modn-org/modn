//! Declared conflicts between catalog releases, checked under the install lock.
//!
//! A package manifest may declare that its mod must not be installed with
//! another mod of the same bucket and game. Each managed catalog package
//! records its declared conflicts in the same commit as its ownership, so the
//! check holds in both directions even after the catalog changes.
//!
//! Dependency readiness treats a managed package with a release's pinned
//! archive digest as that release. Conflicts use the same rule: a conflict
//! matches by catalog identity or by the conflicting release's pinned digest,
//! so a release installed through another manifest with the same archive
//! cannot escape a conflict.
use super::catalog::{CatalogConflict, CatalogReference, CatalogRelease};
use super::validation::validate_transaction_root;
use crate::state::Store;
use anyhow::Result;
use rusqlite::{params, Transaction};
use std::path::Path;

/// One managed package as conflict checks see it.
struct Installed {
    id: String,
    archive_sha256: String,
    reference: Option<CatalogReference>,
}

impl Installed {
    fn label(&self) -> String {
        match &self.reference {
            Some(reference) => format!("{} ({reference})", self.id),
            None => format!("{} (archive {})", self.id, self.archive_sha256),
        }
    }

    fn is(&self, conflict: &CatalogConflict) -> bool {
        self.reference.as_ref() == Some(&conflict.reference)
            || conflict.archive_sha256.as_deref() == Some(self.archive_sha256.as_str())
    }
}

impl Store {
    /// Managed packages in `game` that conflict with `release`, whose archive
    /// digest is `archive_sha256`, as `package-id (bucket/game/mod)`.
    ///
    /// A conflict exists when `release` declares an installed package, or an
    /// installed package declares `release`, by catalog identity or digest.
    /// This read does not lock; installation repeats it under the lock.
    pub fn conflicting_packages(
        &self,
        game: &Path,
        release: &CatalogRelease,
        archive_sha256: &str,
    ) -> Result<Vec<String>> {
        // A read-only database from an older Modn may lack these tables.
        if !self.has_table("catalog_provenance")? {
            return Ok(Vec::new());
        }
        let game = validate_transaction_root(game)?;
        let game_path = game.to_string_lossy().into_owned();
        let mut statement = self.connection.prepare(
            "SELECT p.id, p.archive_sha256, c.bucket, c.game_id, c.mod_id
             FROM managed_packages p LEFT JOIN catalog_provenance c ON c.package_id = p.id
             WHERE p.game_path=?1 ORDER BY p.id",
        )?;
        let installed = statement
            .query_map([&game_path], |row| {
                let bucket: Option<String> = row.get(2)?;
                Ok(Installed {
                    id: row.get(0)?,
                    archive_sha256: row.get(1)?,
                    reference: match bucket {
                        Some(bucket) => {
                            Some(CatalogReference::new(bucket, row.get(3)?, row.get(4)?))
                        }
                        None => None,
                    },
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut declaring = Vec::new();
        if self.has_table("package_conflicts")? {
            let mut statement = self.connection.prepare(
                "SELECT k.package_id FROM package_conflicts k
                 JOIN managed_packages p ON p.id = k.package_id
                 WHERE p.game_path=?1
                   AND ((k.bucket=?2 AND k.game_id=?3 AND k.mod_id=?4) OR k.archive_sha256=?5)",
            )?;
            let reference = &release.reference;
            declaring = statement
                .query_map(
                    params![
                        game_path,
                        reference.bucket,
                        reference.game_id,
                        reference.mod_id,
                        archive_sha256
                    ],
                    |row| row.get::<_, String>(0),
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
        }
        Ok(installed
            .iter()
            .filter(|package| {
                release
                    .conflicts
                    .iter()
                    .any(|conflict| package.is(conflict))
                    || declaring.contains(&package.id)
            })
            .map(Installed::label)
            .collect())
    }

    fn has_table(&self, name: &str) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            [name],
            |row| row.get(0),
        )?)
    }
}

pub(super) fn record(
    tx: &Transaction<'_>,
    package_id: &str,
    release: &CatalogRelease,
) -> Result<()> {
    for conflict in &release.conflicts {
        let reference = &conflict.reference;
        tx.execute(
            "INSERT OR IGNORE INTO package_conflicts
                 (package_id,bucket,game_id,mod_id,archive_sha256)
             VALUES (?1,?2,?3,?4,?5)",
            params![
                package_id,
                reference.bucket,
                reference.game_id,
                reference.mod_id,
                conflict.archive_sha256
            ],
        )?;
    }
    Ok(())
}

pub(super) fn forget(tx: &Transaction<'_>, package_id: &str) -> Result<()> {
    tx.execute(
        "DELETE FROM package_conflicts WHERE package_id=?1",
        [package_id],
    )?;
    Ok(())
}
