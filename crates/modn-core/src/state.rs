use anyhow::{Context, Result};
use rusqlite::{params, Connection, OpenFlags, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Persisted registration for a single game installation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct GameInstallation {
    /// Registered game identifier.
    pub game_id: String,
    /// Registered installation path.
    pub path: PathBuf,
    /// Registration or discovery origin.
    pub source: String,
}

/// Manager-owned state; never stored inside a game's installation directory.
pub struct Store {
    pub(crate) connection: Connection,
    pub(crate) state_root: PathBuf,
}

impl Store {
    /// Open the manager-owned SQLite database and initialize its schema.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("cannot create state directory {}", parent.display()))?;
        }
        let mut connection = Connection::open(path)
            .with_context(|| format!("cannot open state database {}", path.display()))?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS game_installations (
                game_id TEXT NOT NULL,
                path TEXT NOT NULL,
                source TEXT NOT NULL,
                PRIMARY KEY (game_id, path)
            );",
        )?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS managed_packages (
                id TEXT PRIMARY KEY,
                game_path TEXT NOT NULL,
                archive_sha256 TEXT,
                origin TEXT NOT NULL DEFAULT 'archive',
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE IF NOT EXISTS managed_files (
                game_path TEXT NOT NULL,
                path_key TEXT NOT NULL,
                relative_path TEXT NOT NULL,
                package_id TEXT NOT NULL REFERENCES managed_packages(id),
                sha256 TEXT NOT NULL,
                bytes INTEGER NOT NULL,
                PRIMARY KEY(game_path, path_key)
            );
            CREATE TABLE IF NOT EXISTS pending_transactions (
                id TEXT PRIMARY KEY,
                action TEXT NOT NULL,
                status TEXT NOT NULL,
                game_path TEXT NOT NULL,
                package_id TEXT NOT NULL,
                archive_sha256 TEXT
            );
            CREATE TABLE IF NOT EXISTS pending_files (
                transaction_id TEXT NOT NULL REFERENCES pending_transactions(id),
                ord INTEGER NOT NULL,
                relative_path TEXT NOT NULL,
                sha256 TEXT NOT NULL,
                bytes INTEGER NOT NULL,
                PRIMARY KEY(transaction_id, ord)
            );",
        )?;
        // SQLite transactions make metadata-only adoption atomic: no game-file journal
        // is needed because this migration and adoption only change manager state.
        // SQLite's table-rebuild migration must not cascade-delete managed_files.
        // Foreign keys can only be toggled outside a transaction; restore them on
        // both success and failure. The immediate transaction serializes schema
        // inspection with migration, so concurrent openers cannot act on stale metadata.
        let foreign_keys: i64 =
            connection.pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
        connection.pragma_update(None, "foreign_keys", "OFF")?;
        let migration = (|| -> Result<()> {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let (nullable, has_origin): (bool, bool) = {
                let mut stmt = tx.prepare("PRAGMA table_info(managed_packages)")?;
                let columns = stmt
                    .query_map([], |row| {
                        Ok((row.get::<_, String>(1)?, row.get::<_, i64>(3)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                (
                    columns
                        .iter()
                        .any(|(name, nonnull)| name == "archive_sha256" && *nonnull == 0),
                    columns.iter().any(|(name, _)| name == "origin"),
                )
            };
            if !nullable || !has_origin {
                tx.execute_batch(
                    "CREATE TABLE managed_packages_new (
                id TEXT PRIMARY KEY, game_path TEXT NOT NULL,
                archive_sha256 TEXT, origin TEXT NOT NULL DEFAULT 'archive',
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );",
                )?;
                let copy = if has_origin {
                    "INSERT INTO managed_packages_new (id,game_path,archive_sha256,origin,created_at)
                 SELECT id,game_path,archive_sha256,origin,created_at FROM managed_packages"
                } else {
                    "INSERT INTO managed_packages_new (id,game_path,archive_sha256,created_at)
                 SELECT id,game_path,archive_sha256,created_at FROM managed_packages"
                };
                tx.execute(copy, [])?;
                tx.execute_batch(
                    "DROP TABLE managed_packages;
                ALTER TABLE managed_packages_new RENAME TO managed_packages;",
                )?;
                let violated: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_check)",
                    [],
                    |row| row.get(0),
                )?;
                if violated {
                    anyhow::bail!("managed package migration violates foreign key integrity");
                }
            }
            tx.commit()?;
            Ok(())
        })();
        connection.pragma_update(None, "foreign_keys", foreign_keys)?;
        migration?;
        Ok(Self {
            connection,
            state_root: path
                .parent()
                .context("missing state parent")?
                .canonicalize()?,
        })
    }

    /// Read existing registrations without creating or migrating manager state.
    pub fn open_readonly(path: &Path) -> Result<Self> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("cannot read existing state database {}", path.display()))?;
        Ok(Self {
            connection,
            state_root: path
                .parent()
                .context("missing state parent")?
                .canonicalize()?,
        })
    }

    /// Register a game installation using its runtime Luau validation policy.
    pub fn add_game(&self, game_id: &str, path: &Path, source: &str) -> Result<GameInstallation> {
        let path = crate::discovery::validate_installation_for(game_id, path)?;
        let path_text = path.to_str().context("game path must be valid UTF-8")?;
        self.connection.execute(
            "INSERT INTO game_installations (game_id, path, source)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(game_id, path) DO UPDATE SET source=excluded.source",
            params![game_id, path_text, source],
        )?;
        Ok(GameInstallation {
            game_id: game_id.into(),
            path,
            source: source.into(),
        })
    }

    /// List persisted game registrations, ordered by game and path.
    pub fn list(&self) -> Result<Vec<GameInstallation>> {
        let mut statement = self.connection.prepare(
            "SELECT game_id, path, source FROM game_installations ORDER BY game_id, path",
        )?;
        let installations = statement
            .query_map([], |row| {
                Ok(GameInstallation {
                    game_id: row.get(0)?,
                    path: PathBuf::from(row.get::<_, String>(1)?),
                    source: row.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(installations)
    }
}

#[cfg(test)]
mod tests;
