//! Declared conflicts between catalog releases, checked under the install lock.
//!
//! A package manifest may declare that its mod must not be installed with
//! another mod of the same bucket and game. Each managed catalog package
//! records its declared conflicts in the same commit as its ownership, so the
//! check holds in both directions even after the catalog changes.
use super::catalog::{CatalogReference, CatalogRelease};
use super::validation::validate_transaction_root;
use crate::state::Store;
use anyhow::Result;
use rusqlite::{params, Transaction};
use std::path::Path;

impl Store {
    /// Managed packages in `game` that conflict with `release`, in either
    /// direction, as `package-id (bucket/game/mod)`.
    ///
    /// A conflict exists when `release` declares an installed package's
    /// catalog mod, or an installed package declares `release`'s mod. This
    /// read does not lock; installation repeats it under the transaction lock.
    pub fn conflicting_packages(
        &self,
        game: &Path,
        release: &CatalogRelease,
    ) -> Result<Vec<String>> {
        // A read-only database from an older Modn may lack these tables.
        if !self.has_table("catalog_provenance")? {
            return Ok(Vec::new());
        }
        let game = validate_transaction_root(game)?;
        let game_path = game.to_string_lossy().into_owned();
        let mut statement = self.connection.prepare(
            "SELECT p.id, c.bucket, c.game_id, c.mod_id
             FROM managed_packages p JOIN catalog_provenance c ON c.package_id = p.id
             WHERE p.game_path=?1 ORDER BY p.id",
        )?;
        let installed = statement
            .query_map([&game_path], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    CatalogReference::new(row.get(1)?, row.get(2)?, row.get(3)?),
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut found: Vec<String> = installed
            .iter()
            .filter(|(_, reference)| release.conflicts.contains(reference))
            .map(|(id, reference)| format!("{id} ({reference})"))
            .collect();
        if self.has_table("package_conflicts")? {
            let mut statement = self.connection.prepare(
                "SELECT k.package_id FROM package_conflicts k
                 JOIN managed_packages p ON p.id = k.package_id
                 WHERE p.game_path=?1 AND k.bucket=?2 AND k.game_id=?3 AND k.mod_id=?4",
            )?;
            let reference = &release.reference;
            let declaring = statement
                .query_map(
                    params![
                        game_path,
                        reference.bucket,
                        reference.game_id,
                        reference.mod_id
                    ],
                    |row| row.get::<_, String>(0),
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (id, reference) in &installed {
                let entry = format!("{id} ({reference})");
                if declaring.contains(id) && !found.contains(&entry) {
                    found.push(entry);
                }
            }
        }
        found.sort();
        Ok(found)
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
        tx.execute(
            "INSERT OR IGNORE INTO package_conflicts(package_id,bucket,game_id,mod_id)
             VALUES (?1,?2,?3,?4)",
            params![
                package_id,
                conflict.bucket,
                conflict.game_id,
                conflict.mod_id
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
