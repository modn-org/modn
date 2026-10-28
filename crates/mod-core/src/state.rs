use crate::CYBERPUNK_ID;
use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GameInstallation {
    pub game_id: String,
    pub path: PathBuf,
    pub source: String,
}

/// Manager-owned state; never stored inside a game's installation directory.
pub struct Store {
    pub(crate) connection: Connection,
    pub(crate) state_root: PathBuf,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("cannot create state directory {}", parent.display()))?;
        }
        let connection = Connection::open(path)
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
                archive_sha256 TEXT NOT NULL,
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
        Ok(Self {
            connection,
            state_root: path
                .parent()
                .context("missing state parent")?
                .canonicalize()?,
        })
    }

    pub fn add_cyberpunk(&self, path: &Path, source: &str) -> Result<GameInstallation> {
        let path_text = path.to_str().context("game path must be valid UTF-8")?;
        self.connection.execute(
            "INSERT INTO game_installations (game_id, path, source)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(game_id, path) DO UPDATE SET source=excluded.source",
            params![CYBERPUNK_ID, path_text, source],
        )?;
        Ok(GameInstallation {
            game_id: CYBERPUNK_ID.into(),
            path: path.to_owned(),
            source: source.into(),
        })
    }

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
