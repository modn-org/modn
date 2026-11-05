use crate::transactions::{ownership_key, validate_transaction_root};
use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The manager state schema version this build writes.
pub(crate) const SCHEMA_VERSION: i64 = 3;

/// Identity of one root within its installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RootKind {
    Game,
}

impl RootKind {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Game => "game",
        }
    }
}

const OWNERSHIP_TABLES: [(&str, &str, &str); 3] = [
    (
        "managed_files",
        "package_id TEXT NOT NULL REFERENCES managed_packages(id),
         sha256 TEXT NOT NULL, bytes INTEGER NOT NULL",
        "package_id, sha256, bytes",
    ),
    (
        "managed_directories",
        "package_id TEXT NOT NULL REFERENCES managed_packages(id), identity TEXT",
        "package_id, identity",
    ),
    ("created_directories", "identity TEXT", "identity"),
];

/// Rebuild schema v2 ownership tables with root-qualified keys and record
/// every existing installation's game root. Runs inside the caller's
/// transaction, so a failure leaves the v2 state untouched.
pub(super) fn migrate_to_v3(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS installation_roots (
            game_path TEXT NOT NULL,
            root TEXT NOT NULL,
            path TEXT NOT NULL,
            path_key TEXT NOT NULL UNIQUE,
            PRIMARY KEY(game_path, root)
        );",
    )?;
    for (table, columns, copied) in OWNERSHIP_TABLES {
        let has_root: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name='root')",
            [table],
            |row| row.get(0),
        )?;
        if has_root {
            continue;
        }
        connection.execute_batch(&format!(
            "CREATE TABLE {table}_v3 (
                game_path TEXT NOT NULL,
                root TEXT NOT NULL DEFAULT 'game',
                path_key TEXT NOT NULL,
                relative_path TEXT NOT NULL,
                {columns},
                PRIMARY KEY(game_path, root, path_key)
            );
            INSERT INTO {table}_v3(game_path, root, path_key, relative_path, {copied})
                SELECT game_path, 'game', path_key, relative_path, {copied} FROM {table};
            DROP TABLE {table};
            ALTER TABLE {table}_v3 RENAME TO {table};"
        ))?;
    }
    let mut installations = BTreeSet::new();
    for query in [
        "SELECT path FROM game_installations",
        "SELECT game_path FROM managed_packages",
        "SELECT game_path FROM managed_files",
        "SELECT game_path FROM managed_directories",
        "SELECT game_path FROM created_directories",
    ] {
        let mut statement = connection.prepare(query)?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        for row in rows {
            installations.insert(row?);
        }
    }
    for installation in installations {
        let path = Path::new(&installation);
        record_root(connection, path, RootKind::Game, path)?;
    }
    connection.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}

/// Present schema v1 and v2 state through v3 names on a read-only connection.
///
/// Temporary views shadow the stored tables for this connection only and
/// leave the database file unchanged.
pub(super) fn attach_legacy_views(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TEMP VIEW managed_files AS
            SELECT game_path, 'game' AS root, path_key, relative_path, package_id, sha256, bytes
            FROM main.managed_files;
         CREATE TEMP VIEW managed_directories AS
            SELECT game_path, 'game' AS root, path_key, relative_path, package_id, identity
            FROM main.managed_directories;
         CREATE TEMP VIEW created_directories AS
            SELECT game_path, 'game' AS root, path_key, relative_path, identity
            FROM main.created_directories;
         CREATE TEMP VIEW installation_roots AS
            SELECT game_path, 'game' AS root, game_path AS path, NULL AS path_key FROM (
                SELECT path AS game_path FROM main.game_installations
                UNION SELECT game_path FROM main.managed_packages
                UNION SELECT game_path FROM main.managed_files
                UNION SELECT game_path FROM main.managed_directories
                UNION SELECT game_path FROM main.created_directories);",
    )?;
    Ok(())
}

/// Record `path` as the `kind` root of `installation`.
///
/// Refuses a canonical root that another installation or root already
/// claims, and a root whose recorded path differs.
pub(crate) fn record_root(
    connection: &Connection,
    installation: &Path,
    kind: RootKind,
    path: &Path,
) -> Result<()> {
    let installation_text = installation.to_string_lossy();
    let path_text = path.to_string_lossy();
    let key = ownership_key(&path_text);
    let claimant: Option<(String, String)> = connection
        .query_row(
            "SELECT game_path, root FROM installation_roots WHERE path_key=?1",
            [&key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((game_path, root)) = claimant {
        if game_path != installation_text || root != kind.label() {
            bail!(
                "root {path_text} is already the {root} root of installation {game_path}; \
                 two installations cannot share a root"
            );
        }
    }
    connection.execute(
        "INSERT OR IGNORE INTO installation_roots(game_path, root, path, path_key)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            installation_text.as_ref(),
            kind.label(),
            path_text.as_ref(),
            key
        ],
    )?;
    let recorded: String = connection.query_row(
        "SELECT path FROM installation_roots WHERE game_path=?1 AND root=?2",
        params![installation_text.as_ref(), kind.label()],
        |row| row.get(0),
    )?;
    if recorded != path_text {
        bail!(
            "installation {installation_text} already records {} root {recorded}",
            kind.label()
        );
    }
    Ok(())
}

/// Whether `installation` names an installation with recorded roots.
pub(crate) fn is_recorded(connection: &Connection, installation: &Path) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM installation_roots WHERE game_path=?1)",
        [installation.to_string_lossy().as_ref()],
        |row| row.get(0),
    )?)
}

/// Resolve the `kind` root of `installation` through its recorded identity.
///
/// Fails closed when the recorded root is missing, is a link or junction, or
/// no longer canonicalizes to the recorded path. An installation without a
/// recorded game root has committed no ownership yet, so its game root is the
/// canonical form of `installation`.
pub(crate) fn resolve_root(
    connection: &Connection,
    installation: &Path,
    kind: RootKind,
) -> Result<PathBuf> {
    let recorded: Option<String> = connection
        .query_row(
            "SELECT path FROM installation_roots WHERE game_path=?1 AND root=?2",
            params![installation.to_string_lossy().as_ref(), kind.label()],
            |row| row.get(0),
        )
        .optional()?;
    let path = match (recorded, kind) {
        (Some(path), _) => PathBuf::from(path),
        (None, RootKind::Game) => return validate_transaction_root(installation),
    };
    let canonical = validate_transaction_root(&path)?;
    if canonical != path {
        bail!(
            "recorded {} root {} now resolves to {}; it moved or a parent became a link, \
             so Modn refuses to change it",
            kind.label(),
            path.display(),
            canonical.display()
        );
    }
    Ok(path)
}

#[cfg(test)]
#[path = "roots_tests.rs"]
pub(super) mod tests;
