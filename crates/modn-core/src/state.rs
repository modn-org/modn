use anyhow::{bail, Context, Result};
use fs2::FileExt;
use rusqlite::{params, Connection, OpenFlags, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::path::{Component, Path, PathBuf};

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
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.with_file_name(".schema.lock"))?;
        FileExt::lock_exclusive(&lock)?;
        let mut connection = Connection::open(path)
            .with_context(|| format!("cannot open state database {}", path.display()))?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing_state: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master
             WHERE type='table' AND name NOT LIKE 'sqlite_%')",
            [],
            |row| row.get(0),
        )?;
        if existing_state {
            validate_package_schema(&tx)?;
        }
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS game_installations (
                game_id TEXT NOT NULL,
                path TEXT NOT NULL,
                source TEXT NOT NULL,
                PRIMARY KEY (game_id, path)
            );",
        )?;
        tx.execute_batch(
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
            CREATE TABLE IF NOT EXISTS managed_directories (
                game_path TEXT NOT NULL, path_key TEXT NOT NULL,
                relative_path TEXT NOT NULL,
                package_id TEXT NOT NULL REFERENCES managed_packages(id),
                identity TEXT,
                PRIMARY KEY(game_path, path_key)
            );
            CREATE TABLE IF NOT EXISTS pending_directories (
                transaction_id TEXT NOT NULL REFERENCES pending_transactions(id),
                ord INTEGER NOT NULL, relative_path TEXT NOT NULL, identity TEXT,
                PRIMARY KEY(transaction_id, ord)
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
        if !existing_state {
            tx.pragma_update(None, "user_version", 1)?;
        }
        validate_package_schema(&tx)?;
        tx.commit()?;
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
        let lock = File::open(path.with_file_name(".schema.lock"))
            .context("unsupported manager state schema; start with a fresh state directory")?;
        FileExt::lock_shared(&lock)?;
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("cannot read existing state database {}", path.display()))?;
        validate_package_schema(&connection)?;
        Ok(Self {
            connection,
            state_root: path
                .parent()
                .context("missing state parent")?
                .canonicalize()?,
        })
    }

    /// Register a game installation using its runtime Lua validation policy.
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

    /// Unregister one installation without deleting any game files or ownership evidence.
    ///
    /// Refuses missing or ambiguous identities and any managed or pending work.
    pub fn remove_game(&mut self, game_id: &str, path: Option<&Path>) -> Result<GameInstallation> {
        // Use the exact lock held across archive inspection and staging by
        // installs/uninstalls. SQLite row checks alone miss in-flight writes
        // before they insert their pending journal.
        let _guard = crate::transactions::lock_store(self)?;
        let registered = self.list()?;
        let chosen = path.map(canonicalize_removal_path).transpose()?;
        let matches: Vec<_> = registered
            .into_iter()
            .filter(|entry| {
                entry.game_id == game_id && chosen.as_ref().is_none_or(|path| &entry.path == path)
            })
            .collect();
        let entry = match matches.as_slice() {
            [entry] => entry.clone(),
            [] => bail!("no registered installation for {game_id}"),
            _ => bail!("multiple installations registered for {game_id}; specify --path"),
        };
        let path_text = entry
            .path
            .to_str()
            .context("registered game path must be UTF-8")?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let managed: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM managed_packages WHERE game_path=?1)",
            [path_text],
            |row| row.get(0),
        )?;
        let pending: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM pending_transactions WHERE game_path=?1)",
            [path_text],
            |row| row.get(0),
        )?;
        if managed || pending {
            bail!("cannot unregister game with managed packages or pending transactions; uninstall or recover first");
        }
        if tx.execute(
            "DELETE FROM game_installations WHERE game_id=?1 AND path=?2",
            params![entry.game_id, path_text],
        )? != 1
        {
            bail!("game registration changed; retry");
        }
        tx.commit()?;
        Ok(entry)
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

// Canonicalize the nearest existing parent when an uninstalled game folder
// no longer exists. Windows canonical paths may use a verbatim-drive prefix,
// while callers normally supply C:\\... . No missing path is created.
fn canonicalize_removal_path(path: &Path) -> Result<PathBuf> {
    if path
        .components()
        .any(|part| matches!(part, Component::ParentDir))
    {
        bail!("game removal path must not contain parent-directory components");
    }
    let mut current = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut missing = Vec::new();
    loop {
        match current.canonicalize() {
            Ok(mut canonical) => {
                for component in missing.into_iter().rev() {
                    canonical.push(component);
                }
                return Ok(canonical);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = current
                    .file_name()
                    .context("invalid missing game directory")?;
                missing.push(name.to_os_string());
                current = current
                    .parent()
                    .context("cannot resolve game directory")?
                    .to_path_buf();
            }
            Err(error) => return Err(error.into()),
        }
    }
}

/// Reject old ownership ledgers rather than interpreting manually claimed files
/// as ZIP-installed packages. A clean installation starts with a fresh state DB.
fn validate_package_schema(connection: &Connection) -> Result<()> {
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version != 1 {
        bail!("unsupported manager state schema; start with a fresh state directory");
    }
    let (archive_required, manual_origin): (Option<i64>, bool) = connection.query_row(
        r#"SELECT
            (SELECT "notnull" FROM pragma_table_info('managed_packages')
             WHERE name='archive_sha256'),
            EXISTS(SELECT 1 FROM pragma_table_info('managed_packages') WHERE name='origin')"#,
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if archive_required != Some(1) || manual_origin {
        bail!("unsupported manager state schema; start with a fresh state directory");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
