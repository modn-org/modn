//! Read-only package and directory ownership lookups.
use super::journal::{ManagedPackage, OwnedFile};
use super::validation::validate_transaction_root;
use crate::state::Store;
use anyhow::{Context, Result};
use rusqlite::params;
use std::path::Path;

impl Store {
    /// List packages installed by this manager for the selected game.
    pub fn installed(&self, game: &Path) -> Result<Vec<ManagedPackage>> {
        let game = validate_transaction_root(game)?;
        let name = game.to_string_lossy().into_owned();
        // Read-only previews must support PR14 ledgers without migrating them.
        let has_origin: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('managed_packages')
             WHERE name='origin')",
            [],
            |row| row.get(0),
        )?;
        let statement = if has_origin {
            "SELECT id, archive_sha256, origin FROM managed_packages
             WHERE game_path=?1 ORDER BY created_at, id"
        } else {
            "SELECT id, archive_sha256, 'archive' AS origin FROM managed_packages
             WHERE game_path=?1 ORDER BY created_at, id"
        };
        let mut query = self.connection.prepare(statement)?;
        let packages = query
            .query_map([&name], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        packages.into_iter().map(|(id, archive_sha256, origin)| {
            let mut statement = self.connection.prepare(
                "SELECT relative_path, sha256, bytes FROM managed_files WHERE package_id=?1 ORDER BY relative_path")?;
            let files = statement.query_map([&id], |row| Ok(OwnedFile {
                destination: row.get(0)?, sha256: row.get(1)?, bytes: row.get::<_,i64>(2)? as u64
            }))?.collect::<rusqlite::Result<Vec<_>>>()?;
            let mut directories = Vec::new();
            let has_dirs: bool = self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='managed_directories')",
                [], |row| row.get(0))?;
            if has_dirs {
                let mut q = self.connection.prepare(
                    "SELECT relative_path FROM managed_directories WHERE package_id=?1 ORDER BY relative_path")?;
                directories = q.query_map([&id], |row| row.get(0))?
                    .collect::<rusqlite::Result<Vec<String>>>()?;
            }
            Ok(ManagedPackage { id, game_installation: game.clone(), archive_sha256, origin, files, directories })
        }).collect()
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

    /// Native identity recorded for a successfully installed exclusive folder.
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
