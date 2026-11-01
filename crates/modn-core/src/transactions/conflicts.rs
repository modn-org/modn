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
    /// Managed packages in `game` that conflict with an archive whose digest
    /// is `archive_sha256`, as `package-id (bucket/game/mod)`.
    ///
    /// A conflict exists when `release` declares an installed package, or an
    /// installed package declares the archive's release, by catalog identity
    /// or digest. A standalone archive (`release` is `None`) is checked by
    /// digest against recorded conflicts only. This read does not lock;
    /// installation repeats it under the lock.
    pub fn conflicting_packages(
        &self,
        game: &Path,
        release: Option<&CatalogRelease>,
        archive_sha256: &str,
    ) -> Result<Vec<String>> {
        let game = validate_transaction_root(game)?;
        let game_path = game.to_string_lossy().into_owned();
        // A read-only database from an older Modn may lack the catalog
        // tables; its packages still match by archive digest.
        let mut statement = self
            .connection
            .prepare(if self.has_table("catalog_provenance")? {
                "SELECT p.id, p.archive_sha256, c.bucket, c.game_id, c.mod_id
             FROM managed_packages p LEFT JOIN catalog_provenance c ON c.package_id = p.id
             WHERE p.game_path=?1 ORDER BY p.id"
            } else {
                "SELECT id, archive_sha256, NULL, NULL, NULL
             FROM managed_packages WHERE game_path=?1 ORDER BY id"
            })?;
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
            // A standalone archive has no catalog identity: NULL never matches.
            let reference = release.map(|release| &release.reference);
            declaring = statement
                .query_map(
                    params![
                        game_path,
                        reference.map(|r| r.bucket.as_str()),
                        reference.map(|r| r.game_id.as_str()),
                        reference.map(|r| r.mod_id.as_str()),
                        archive_sha256
                    ],
                    |row| row.get::<_, String>(0),
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
        }
        let declared: &[CatalogConflict] = release.map_or(&[], |release| &release.conflicts);
        Ok(installed
            .iter()
            .filter(|package| {
                declared.iter().any(|conflict| package.is(conflict))
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
