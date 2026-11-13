use crate::progress::OperationControl;
use anyhow::{bail, Context, Result};
use fs2::FileExt;
use rusqlite::{params, Connection, OpenFlags, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::path::{Component, Path, PathBuf};

/// Persisted registration for a single game installation.
#[expect(missing_docs, reason = "Field and variant names state their meaning")]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct GameInstallation {
    pub game_id: String,
    pub path: PathBuf,
    pub source: String,
}

/// Manager-owned state; never stored inside a game's installation directory.
pub struct Store {
    pub(crate) connection: Connection,
    pub(crate) state_root: PathBuf,
    pub(crate) control: OperationControl,
    pub(crate) history: Option<crate::history::HistoryContext>,
}

impl Store {
    /// Report the progress of later operations to `control` and let its
    /// token cancel them before their commit point.
    ///
    /// Install, update, stack install and uninstall use it. A new store
    /// uses [`OperationControl::default`], which reports nothing.
    pub fn set_control(&mut self, control: OperationControl) {
        self.control = control;
    }

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
            );
            CREATE TABLE IF NOT EXISTS catalog_provenance (
                package_id TEXT PRIMARY KEY REFERENCES managed_packages(id),
                bucket TEXT NOT NULL,
                game_id TEXT NOT NULL,
                mod_id TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS package_dependencies (
                package_id TEXT NOT NULL REFERENCES managed_packages(id),
                dependency_id TEXT NOT NULL REFERENCES managed_packages(id),
                PRIMARY KEY(package_id, dependency_id)
            );
            CREATE TABLE IF NOT EXISTS pending_created_directories (
                transaction_id TEXT NOT NULL REFERENCES pending_transactions(id),
                ord INTEGER NOT NULL,
                relative_path TEXT NOT NULL,
                identity TEXT,
                PRIMARY KEY(transaction_id, ord)
            );
            CREATE TABLE IF NOT EXISTS created_directories (
                game_path TEXT NOT NULL,
                path_key TEXT NOT NULL,
                relative_path TEXT NOT NULL,
                identity TEXT,
                PRIMARY KEY(game_path, path_key)
            );
            CREATE TABLE IF NOT EXISTS package_conflicts (
                package_id TEXT NOT NULL REFERENCES managed_packages(id),
                bucket TEXT NOT NULL,
                game_id TEXT NOT NULL,
                mod_id TEXT NOT NULL,
                archive_sha256 TEXT,
                PRIMARY KEY(package_id, bucket, game_id, mod_id)
            );
            CREATE TABLE IF NOT EXISTS package_requirements (
                package_id TEXT NOT NULL REFERENCES managed_packages(id),
                bucket TEXT NOT NULL,
                game_id TEXT NOT NULL,
                mod_id TEXT NOT NULL,
                archive_sha256 TEXT NOT NULL,
                requirement TEXT,
                PRIMARY KEY(package_id, bucket, game_id, mod_id)
            );
            CREATE TABLE IF NOT EXISTS pending_links (
                transaction_id TEXT NOT NULL REFERENCES pending_transactions(id),
                ord INTEGER NOT NULL,
                PRIMARY KEY(transaction_id, ord)
            );
            CREATE TABLE IF NOT EXISTS link_recorded_journals (
                transaction_id TEXT PRIMARY KEY REFERENCES pending_transactions(id)
            );
            CREATE TABLE IF NOT EXISTS pending_stacks (
                id TEXT PRIMARY KEY,
                game_path TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS pending_stack_packages (
                stack_id TEXT NOT NULL REFERENCES pending_stacks(id),
                ord INTEGER NOT NULL,
                package_id TEXT NOT NULL UNIQUE REFERENCES managed_packages(id),
                PRIMARY KEY(stack_id, ord)
            );
            CREATE TABLE IF NOT EXISTS managed_package_releases (
                package_id TEXT PRIMARY KEY REFERENCES managed_packages(id),
                mod_id TEXT NOT NULL,
                version TEXT,
                source TEXT,
                archive_blake3 TEXT,
                catalog TEXT,
                declarations TEXT
            );
            CREATE TABLE IF NOT EXISTS history_operations (
                seq INTEGER PRIMARY KEY AUTOINCREMENT,
                id TEXT NOT NULL UNIQUE,
                kind TEXT NOT NULL,
                game_id TEXT NOT NULL,
                installation TEXT NOT NULL,
                recorded_at TEXT NOT NULL,
                undoes TEXT
            );
            CREATE TABLE IF NOT EXISTS history_packages (
                operation_id TEXT NOT NULL,
                ord INTEGER NOT NULL,
                mod_id TEXT NOT NULL,
                package_id_before TEXT,
                package_id_after TEXT,
                release_before TEXT,
                release_after TEXT,
                PRIMARY KEY(operation_id, ord)
            );
            CREATE TABLE IF NOT EXISTS history_files (
                operation_id TEXT NOT NULL,
                package_ord INTEGER NOT NULL,
                ord INTEGER NOT NULL,
                root TEXT NOT NULL,
                relative_path TEXT NOT NULL,
                digest_before TEXT,
                digest_after TEXT,
                PRIMARY KEY(operation_id, package_ord, ord)
            );",
        )?;
        if !existing_state {
            tx.pragma_update(None, "user_version", 1)?;
        }
        validate_package_schema(&tx)?;
        let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version == 1 {
            tx.execute_batch(
                "ALTER TABLE managed_packages ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;",
            )?;
            tx.pragma_update(None, "user_version", 2)?;
        }
        if version < roots::SCHEMA_VERSION {
            roots::migrate_to_v3(&tx)?;
        }
        migrate_ownership_keys(&tx)?;
        crate::approval::ensure_schema(&tx)?;
        ensure_history_undoes(&tx)?;
        crate::history::ensure_release_schema(&tx)?;
        tx.commit()?;
        Ok(Self {
            connection,
            state_root: path
                .parent()
                .context("missing state parent")?
                .canonicalize()?,
            control: OperationControl::default(),
            history: None,
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
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version < roots::SCHEMA_VERSION {
            roots::attach_legacy_views(&connection)?;
        }
        crate::approval::attach_missing_view(&connection)?;
        Ok(Self {
            connection,
            state_root: path
                .parent()
                .context("missing state parent")?
                .canonicalize()?,
            control: OperationControl::default(),
            history: None,
        })
    }

    /// Register a game installation using its runtime Lua validation policy.
    pub fn add_game(&self, game_id: &str, path: &Path, source: &str) -> Result<GameInstallation> {
        let path = crate::discovery::validate_installation_for(game_id, path)?;
        let path_text = path.to_str().context("game path must be valid UTF-8")?;
        let tx = self.connection.unchecked_transaction()?;
        roots::record_root(&tx, &path, roots::RootKind::Game, &path)?;
        tx.execute(
            "INSERT INTO game_installations (game_id, path, source)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(game_id, path) DO UPDATE SET source=excluded.source",
            params![game_id, path_text, source],
        )?;
        tx.commit()?;
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
            "SELECT EXISTS(SELECT 1 FROM pending_transactions WHERE game_path=?1)
                 OR EXISTS(SELECT 1 FROM pending_stacks WHERE game_path=?1)",
            [path_text],
            |row| row.get(0),
        )?;
        if managed || pending {
            bail!("cannot unregister game with managed packages or pending transactions; uninstall or recover first");
        }
        tx.execute(
            "DELETE FROM created_directories WHERE game_path=?1",
            [path_text],
        )?;
        if tx.execute(
            "DELETE FROM game_installations WHERE game_id=?1 AND path=?2",
            params![entry.game_id, path_text],
        )? != 1
        {
            bail!("game registration changed; retry");
        }
        tx.execute(
            "DELETE FROM installation_roots WHERE game_path=?1
             AND NOT EXISTS(SELECT 1 FROM game_installations WHERE path=?1)",
            [path_text],
        )?;
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

fn migrate_ownership_keys(connection: &Connection) -> Result<()> {
    use std::collections::BTreeMap;

    for table in [
        "managed_files",
        "managed_directories",
        "created_directories",
    ] {
        let path_key = if table == "created_directories" {
            crate::transactions::directory_candidate_key
        } else {
            crate::transactions::ownership_key
        };
        let mut query = connection.prepare(&format!(
            "SELECT game_path, root, path_key, relative_path FROM {table}
             ORDER BY game_path, root, path_key"
        ))?;
        let rows = query
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut keys = BTreeMap::new();
        for (game, root, _, relative) in &rows {
            let key = path_key(relative);
            if let Some(previous) = keys.insert((game, root, key), relative) {
                bail!("ownership key migration collision in {table} for game {game}: {previous:?} and {relative:?}; state unchanged");
            }
        }
        for (game, root, old_key, relative) in rows {
            let key = path_key(&relative);
            if key != old_key {
                connection.execute(
                    &format!(
                        "UPDATE {table} SET path_key=?1
                         WHERE game_path=?2 AND root=?3 AND path_key=?4"
                    ),
                    params![key, game, root, old_key],
                )?;
            }
        }
    }
    Ok(())
}

fn ensure_history_undoes(connection: &Connection) -> Result<()> {
    let present: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('history_operations') WHERE name='undoes')",
        [],
        |row| row.get(0),
    )?;
    if !present {
        connection.execute_batch("ALTER TABLE history_operations ADD COLUMN undoes TEXT;")?;
    }
    Ok(())
}

fn validate_package_schema(connection: &Connection) -> Result<()> {
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let latest = roots::SCHEMA_VERSION.max(crate::approval::SCHEMA_VERSION);
    if !(1..=latest).contains(&version) {
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

mod roots;
#[cfg(test)]
pub(crate) use roots::tests::{link_directory, unlink_directory};
pub(crate) use roots::{
    check_root, is_recorded, record_root, recorded_root, resolve_root, RootKind,
};

#[cfg(test)]
mod tests;
