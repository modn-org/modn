//! Immutable local versions and transactional selection, independent of game ownership.
use super::loader;
use super::package::{ModulePackage as Package, PackageInfo};
use super::store_files::{check_path, publish, read_package, sync_directory};
use anyhow::{bail, Context, Result};
use fs2::FileExt;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::Serialize;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;

/// Separate module repository beneath the manager's state directory.
#[derive(Debug)]
pub struct ModuleStore {
    root: PathBuf,
}

#[derive(Clone, Copy)]
enum InstallMode {
    Inactive,
    Update,
}

/// Persisted opt-in and rollback information; disabled versions remain installed.
#[derive(Debug, Serialize)]
#[non_exhaustive]
pub struct ModuleStatus {
    /// Requested game identity.
    pub game_id: String,
    /// Whether a user selection is enabled.
    pub enabled: bool,
    /// Last explicitly selected package, including while disabled.
    pub selected: Option<PackageInfo>,
    /// Previous selection, retained for reversible rollback.
    pub previous: Option<PackageInfo>,
}

impl ModuleStore {
    /// Construct a handle without creating directories, locks or databases.
    #[must_use]
    pub fn new(state_dir: &Path) -> Self {
        Self {
            root: state_dir.join("game-modules"),
        }
    }

    fn database(&self) -> PathBuf {
        self.root.join("modules.sqlite")
    }

    fn readonly(&self) -> Result<Option<Connection>> {
        let path = self.database();
        check_path(&path)?;
        if !path.try_exists()? {
            return Ok(None);
        }
        let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        db.busy_timeout(Duration::from_secs(5))?;
        if uninitialized(&db)? {
            return Ok(None);
        }
        schema_version(&db)?;
        Ok(Some(db))
    }

    fn writer(&self) -> Result<(File, Connection)> {
        check_path(&self.database())?;
        for suffix in ["-journal", "-wal", "-shm"] {
            check_path(&self.root.join(format!("modules.sqlite{suffix}")))?;
        }
        check_path(&self.root.join(".lock"))?;
        fs::create_dir_all(&self.root)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join(".lock"))?;
        lock.try_lock_exclusive()
            .context("module store busy; retry after the other module command finishes")?;
        let db = Connection::open(self.database())?;
        db.busy_timeout(Duration::from_secs(5))?;
        db.execute_batch("PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;")?;
        let version: u32 = db.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version == 0 {
            if !uninitialized(&db)? {
                bail!("unrecognized module store database");
            }
            db.execute_batch("BEGIN IMMEDIATE;
                CREATE TABLE packages (game_id TEXT NOT NULL, version TEXT NOT NULL,
                    api_version INTEGER NOT NULL, entrypoint TEXT NOT NULL, sha256 TEXT PRIMARY KEY,
                    manifest_sha256 TEXT NOT NULL, script_sha256 TEXT NOT NULL, UNIQUE(game_id, version));
                CREATE TABLE selections (game_id TEXT PRIMARY KEY, active TEXT NOT NULL REFERENCES packages(sha256),
                    previous TEXT REFERENCES packages(sha256), enabled INTEGER NOT NULL CHECK(enabled IN (0,1)));
                PRAGMA user_version=1; COMMIT;")?;
            sync_directory(&self.root)?;
        }
        schema_version(&db)?;
        Ok((lock, db))
    }

    // Read-only commands never recover a hot journal. An approved mutation can
    // restore the last committed database under the writer lock before retrying.
    fn before_change(&self) -> Result<Option<Connection>> {
        match self.readonly() {
            Err(_) if self.root.join("modules.sqlite-journal").is_file() => {
                let (_lock, db) = self.writer()?;
                drop(db);
                self.readonly()
            }
            result => result,
        }
    }

    /// List and integrity-check installed versions without initializing state.
    pub fn list(&self) -> Result<Vec<PackageInfo>> {
        let Some(db) = self.readonly()? else {
            return Ok(Vec::new());
        };
        let mut statement = db.prepare("SELECT game_id,version,api_version,entrypoint,sha256,manifest_sha256,script_sha256 FROM packages ORDER BY game_id,version")?;
        let packages = statement
            .query_map([], package_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for package in &packages {
            read_package(&self.root, package)?;
        }
        Ok(packages)
    }

    /// Inspect selected and previous versions, failing closed on stored corruption.
    pub fn status(&self, game: &str) -> Result<ModuleStatus> {
        check_game(game)?;
        let Some(db) = self.readonly()? else {
            return Ok(ModuleStatus {
                game_id: game.into(),
                enabled: false,
                selected: None,
                previous: None,
            });
        };
        let status = status_in(&db, game)?;
        for package in [&status.selected, &status.previous].into_iter().flatten() {
            read_package(&self.root, package)?;
        }
        Ok(status)
    }

    pub(super) fn active(&self, game: &str) -> Result<Option<Package>> {
        check_game(game)?;
        let Some(db) = self.readonly()? else {
            return Ok(None);
        };
        let status = status_in(&db, game)?;
        if !status.enabled {
            return Ok(None);
        }
        let package = status
            .selected
            .context("enabled module has no selected version")?;
        read_package(&self.root, &package)
            .map(Some)
            .context("active user module is corrupt; automatic fallback refused")
    }

    /// Install a validated local ZIP, retaining existing content. This never enables it.
    pub fn install(&self, package: &Package) -> Result<PackageInfo> {
        self.preflight(&package.info, InstallMode::Inactive)?;
        let (_lock, mut db) = self.writer()?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        ensure_identity(&tx, &package.info)?;
        publish(&self.root, package)?;
        insert(&tx, &package.info)?;
        tx.commit()?;
        Ok(package.info.clone())
    }

    /// Check install conflicts and any existing target without initializing or recovering state.
    pub fn check_install(&self, package: &Package) -> Result<()> {
        if let Some(db) = self.readonly()? {
            ensure_identity(&db, &package.info)?;
        }
        let target = self.root.join("versions").join(&package.info.sha256);
        check_path(&target)?;
        if target.try_exists()? {
            read_package(&self.root, &package.info)?;
        }
        Ok(())
    }

    /// Install and select an update in one database transaction. Requires prior enablement.
    pub fn update(&self, package: &Package) -> Result<PackageInfo> {
        self.preflight(&package.info, InstallMode::Update)?;
        let (_lock, mut db) = self.writer()?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current = status_in(&tx, &package.info.game_id)?;
        if !current.enabled {
            bail!("update requires an enabled user module; install then enable explicitly");
        }
        let active = current.selected.context("missing active module")?;
        read_package(&self.root, &active)?;
        ensure_identity(&tx, &package.info)?;
        publish(&self.root, package)?;
        insert(&tx, &package.info)?;
        select(&tx, &package.info, Some(&active.sha256))?;
        tx.commit()?;
        Ok(package.info.clone())
    }

    fn preflight(&self, info: &PackageInfo, mode: InstallMode) -> Result<()> {
        if let Some(db) = self.before_change()? {
            ensure_identity(&db, info)?;
            if matches!(mode, InstallMode::Update) {
                let current = status_in(&db, &info.game_id)?;
                if !current.enabled {
                    bail!("update requires an enabled user module");
                }
                read_package(
                    &self.root,
                    &current.selected.context("missing active version")?,
                )?;
            }
        } else if matches!(mode, InstallMode::Update) {
            bail!("update requires an enabled user module; install and enable first");
        }
        Ok(())
    }

    /// Explicitly select an installed version. A disabled selection can be re-enabled.
    pub fn enable(&self, game: &str, version: &str) -> Result<()> {
        check_game(game)?;
        let db = self
            .before_change()?
            .context("no installed module versions")?;
        let target = by_version(&db, game, version)?;
        read_package(&self.root, &target)?;
        drop(db);
        let (_lock, mut db) = self.writer()?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let target = by_version(&tx, game, version)?;
        read_package(&self.root, &target)?;
        let current = status_in(&tx, game)?;
        // Explicit enable is also a repair route when the old active source is corrupt.
        let previous = current.selected.as_ref().map(|p| p.sha256.as_str());
        select(&tx, &target, previous)?;
        tx.commit()?;
        Ok(())
    }

    /// Disable user policy resolution, preserving both selected and rollback versions.
    pub fn disable(&self, game: &str) -> Result<()> {
        check_game(game)?;
        let db = self
            .before_change()?
            .context("no module selection to disable")?;
        if status_in(&db, game)?.selected.is_none() {
            bail!("no module selection to disable");
        }
        drop(db);
        let (_lock, mut db) = self.writer()?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("UPDATE selections SET enabled=0 WHERE game_id=?1", [game])?;
        tx.commit()?;
        Ok(())
    }

    /// Swap current and previous versions, validating the target before any write.
    pub fn rollback(&self, game: &str) -> Result<()> {
        drop(self.before_change()?);
        let status = self.status(game)?;
        if !status.enabled || status.previous.is_none() {
            bail!("no enabled rollback target");
        }
        let (_lock, mut db) = self.writer()?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current = status_in(&tx, game)?;
        if !current.enabled {
            bail!("module was disabled; retry after explicit enablement");
        }
        let target = current.previous.context("no rollback target")?;
        read_package(&self.root, &target)?;
        select(
            &tx,
            &target,
            current.selected.as_ref().map(|p| p.sha256.as_str()),
        )?;
        tx.commit()?;
        Ok(())
    }
}

fn check_game(game: &str) -> Result<()> {
    if !loader::valid_game_id(game) {
        bail!("invalid module game ID");
    }
    Ok(())
}

fn schema_version(db: &Connection) -> Result<()> {
    let version: u32 = db.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version != 1 {
        bail!("unsupported module store schema version {version}");
    }
    Ok(())
}

fn uninitialized(db: &Connection) -> Result<bool> {
    let version: u32 = db.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let tables: u32 = db.query_row(
        "SELECT count(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
        [],
        |row| row.get(0),
    )?;
    Ok(version == 0 && tables == 0)
}

fn package_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PackageInfo> {
    Ok(PackageInfo {
        game_id: row.get(0)?,
        version: row.get(1)?,
        api_version: row.get(2)?,
        entrypoint: row.get(3)?,
        sha256: row.get(4)?,
        manifest_sha256: row.get(5)?,
        script_sha256: row.get(6)?,
    })
}

fn by_version(db: &Connection, game: &str, version: &str) -> Result<PackageInfo> {
    db.query_row("SELECT game_id,version,api_version,entrypoint,sha256,manifest_sha256,script_sha256 FROM packages WHERE game_id=?1 AND version=?2",
        params![game, version], package_row).context("module version is not installed")
}

fn status_in(db: &Connection, game: &str) -> Result<ModuleStatus> {
    let selection = db
        .query_row(
            "SELECT active,previous,enabled FROM selections WHERE game_id=?1",
            [game],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, bool>(2)?,
                ))
            },
        )
        .optional()?;
    let lookup = |digest: &str| -> Result<PackageInfo> {
        let package = db.query_row("SELECT game_id,version,api_version,entrypoint,sha256,manifest_sha256,script_sha256 FROM packages WHERE sha256=?1", [digest], package_row)?;
        if package.game_id != game {
            bail!("module selection game identity mismatch");
        }
        Ok(package)
    };
    let (selected, previous, enabled) = match selection {
        Some((active, previous, enabled)) => (
            Some(lookup(&active)?),
            previous.as_deref().map(lookup).transpose()?,
            enabled,
        ),
        None => (None, None, false),
    };
    Ok(ModuleStatus {
        game_id: game.into(),
        enabled,
        selected,
        previous,
    })
}

fn ensure_identity(db: &Connection, package: &PackageInfo) -> Result<()> {
    let digest: Option<String> = db
        .query_row(
            "SELECT sha256 FROM packages WHERE game_id=?1 AND version=?2",
            params![package.game_id, package.version],
            |row| row.get(0),
        )
        .optional()?;
    if digest.is_some_and(|digest| digest != package.sha256) {
        bail!("module version already installed with different content; use a new version label");
    }
    Ok(())
}

fn insert(db: &Connection, p: &PackageInfo) -> Result<()> {
    db.execute(
        "INSERT OR IGNORE INTO packages VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![
            p.game_id,
            p.version,
            p.api_version,
            p.entrypoint,
            p.sha256,
            p.manifest_sha256,
            p.script_sha256
        ],
    )?;
    Ok(())
}

fn select(db: &Connection, package: &PackageInfo, previous: Option<&str>) -> Result<()> {
    if previous == Some(package.sha256.as_str()) {
        db.execute(
            "UPDATE selections SET enabled=1 WHERE game_id=?1",
            [&package.game_id],
        )?;
    } else {
        db.execute("INSERT INTO selections VALUES (?1,?2,?3,1) ON CONFLICT(game_id) DO UPDATE SET active=excluded.active, previous=excluded.previous, enabled=1",
            params![package.game_id, package.sha256, previous])?;
    }
    Ok(())
}
