use super::journal::{ManagedPackage, OwnedFile};
use super::validation::validate_transaction_root;
use crate::state::Store;
use anyhow::{Context, Result};
use rusqlite::params;
use std::path::Path;

#[expect(missing_docs, reason = "Method names state their behavior")]
impl Store {
    pub fn installed(&self, game: &Path) -> Result<Vec<ManagedPackage>> {
        let game = validate_transaction_root(game)?;
        let name = game.to_string_lossy().into_owned();
        let mut query = self.connection.prepare(
            "SELECT id, archive_sha256 FROM managed_packages
             WHERE game_path=?1 ORDER BY created_at, id",
        )?;
        let packages = query
            .query_map([&name], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        packages.into_iter().map(|(id, archive_sha256)| {
            let mut statement = self.connection.prepare(
                "SELECT relative_path, sha256, bytes FROM managed_files WHERE package_id=?1 ORDER BY relative_path")?;
            let files = statement.query_map([&id], |row| Ok(OwnedFile {
                destination: row.get(0)?, sha256: row.get(1)?, bytes: row.get::<_,i64>(2)? as u64
            }))?.collect::<rusqlite::Result<Vec<_>>>()?;
            let mut query = self.connection.prepare(
                "SELECT relative_path FROM managed_directories WHERE package_id=?1 ORDER BY relative_path",
            )?;
            let directories = query
                .query_map([&id], |row| row.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?;
            let catalog = super::catalog::reference_of(self, &id)?;
            Ok(ManagedPackage { id, game_installation: game.clone(), archive_sha256, files, directories, catalog })
        }).collect()
    }

    /// Game-relative folders Modn created for its packages in `game`.
    ///
    /// These are the parents of owned files that did not exist before an
    /// install. They belong to no single package: an uninstall removes each
    /// one when it is empty, and keeps it while other content remains.
    pub fn created_directories(&self, game: &Path) -> Result<Vec<String>> {
        let game = validate_transaction_root(game)?;
        let mut query = self.connection.prepare(
            "SELECT relative_path FROM created_directories
             WHERE game_path=?1 ORDER BY path_key",
        )?;
        let directories = query
            .query_map([game.to_string_lossy().as_ref()], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(directories)
    }

    pub(crate) fn verified_managed_directory(
        &self,
        package_id: &str,
        root: &str,
        path: &Path,
    ) -> bool {
        self.managed_directory_identity(package_id, root)
            .and_then(|id| super::directory::verify_directory(path, &id))
            .is_ok()
    }

    pub(super) fn managed_directory_identity(
        &self,
        package_id: &str,
        root: &str,
    ) -> Result<String> {
        self.connection
            .query_row(
                "SELECT identity FROM managed_directories WHERE package_id=?1 AND relative_path=?2",
                params![package_id, root],
                |row| row.get::<_, Option<String>>(0),
            )?
            .context("exclusive directory has no recorded identity; refusing removal")
    }
}
