//! Profile, file-claim and profile-work tables of the state database.
//!
//! `Store::open` runs [`DDL`] in the fresh-database schema batch, after the
//! ownership tables. There is no migration: an existing database at an earlier
//! schema version is refused before any of these tables is read. Profile
//! lifecycle, claim and switch SQL belongs in this module.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

/// The tables that schema version 8 adds, for layout validation.
pub(super) const TABLES: [&str; 7] = [
    "profiles",
    "profile_packages",
    "file_claims",
    "pending_overrides",
    "pending_restores",
    "pending_profile_switches",
    "history_profile_switches",
];

/// Every table and index that schema version 8 adds.
///
/// `managed_files` gains `profile_id` and `archive_entry` in the batch in
/// `state.rs`, because its `CREATE TABLE` lives there. Deleting a profile
/// removes its memberships through `ON DELETE CASCADE`; the later profile code
/// still deletes them explicitly, because Modn does not enable foreign keys.
pub(super) const DDL: &str = "\
CREATE TABLE IF NOT EXISTS profiles (
    id TEXT PRIMARY KEY,
    game_path TEXT NOT NULL,
    name TEXT NOT NULL,
    active INTEGER NOT NULL DEFAULT 0 CHECK (active IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (game_path, name)
);
CREATE UNIQUE INDEX IF NOT EXISTS profiles_one_active_per_game
    ON profiles(game_path) WHERE active = 1;

CREATE TABLE IF NOT EXISTS profile_packages (
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    package_id TEXT NOT NULL REFERENCES managed_packages(id),
    priority INTEGER NOT NULL CHECK (priority >= 0),
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    PRIMARY KEY (profile_id, package_id),
    UNIQUE (profile_id, priority)
);

CREATE TABLE IF NOT EXISTS file_claims (
    package_id TEXT NOT NULL REFERENCES managed_packages(id),
    game_path TEXT NOT NULL,
    root TEXT NOT NULL DEFAULT 'game',
    path_key TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    archive_entry TEXT NOT NULL,
    blake3 TEXT NOT NULL,
    bytes INTEGER NOT NULL,
    PRIMARY KEY (package_id, game_path, root, path_key)
);
CREATE INDEX IF NOT EXISTS file_claims_by_path
    ON file_claims(game_path, root, path_key);

CREATE TABLE IF NOT EXISTS pending_overrides (
    transaction_id TEXT NOT NULL REFERENCES pending_transactions(id),
    ord INTEGER NOT NULL,
    root TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    package_id TEXT NOT NULL,
    blake3 TEXT NOT NULL,
    bytes INTEGER NOT NULL,
    PRIMARY KEY (transaction_id, ord)
);

CREATE TABLE IF NOT EXISTS pending_restores (
    transaction_id TEXT NOT NULL REFERENCES pending_transactions(id),
    ord INTEGER NOT NULL,
    package_id TEXT NOT NULL,
    archive_entry TEXT NOT NULL,
    blake3 TEXT NOT NULL,
    bytes INTEGER NOT NULL,
    PRIMARY KEY (transaction_id, ord)
);

CREATE TABLE IF NOT EXISTS pending_profile_switches (
    transaction_id TEXT PRIMARY KEY REFERENCES pending_transactions(id),
    game_path TEXT NOT NULL,
    from_profile TEXT NOT NULL,
    to_profile TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS history_profile_switches (
    operation_id TEXT PRIMARY KEY,
    game_path TEXT NOT NULL,
    from_profile TEXT NOT NULL,
    to_profile TEXT NOT NULL
);
";

/// The profile that owns new mainstream installs for one installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActiveProfile {
    pub(crate) id: String,
}

/// The active profile of `game`, creating `default` when the installation has
/// no profile yet.
pub(crate) fn ensure_profile(connection: &Connection, game: &Path) -> Result<ActiveProfile> {
    let game_text = game.to_string_lossy();
    let active: Option<String> = connection
        .query_row(
            "SELECT id FROM profiles WHERE game_path=?1 AND active=1",
            [game_text.as_ref()],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = active {
        return Ok(ActiveProfile { id });
    }
    let existing_default: Option<String> = connection
        .query_row(
            "SELECT id FROM profiles WHERE game_path=?1 AND name='default'",
            [game_text.as_ref()],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = existing_default {
        connection.execute("UPDATE profiles SET active=1 WHERE id=?1", [&id])?;
        return Ok(ActiveProfile { id });
    }
    let id = profile_id(&game_text, "default");
    connection.execute(
        "INSERT INTO profiles(id,game_path,name,active) VALUES (?1,?2,'default',1)",
        params![id, game_text.as_ref()],
    )?;
    Ok(ActiveProfile { id })
}

/// A stable profile identifier derived from the installation path and name.
pub(crate) fn profile_id(game_path: &str, name: &str) -> String {
    let digest = blake3::hash(format!("{game_path}\0{name}").as_bytes());
    format!("profile-{}", digest.to_hex())
}

/// Append `package_id` to `profile_id` at the next priority and enable it.
pub(crate) fn append_package(
    connection: &Connection,
    profile_id: &str,
    package_id: &str,
) -> Result<i64> {
    connection.execute(
        "INSERT INTO profile_packages(profile_id,package_id,priority,enabled)
         SELECT ?1,?2,COALESCE(MAX(priority),-1)+1,1
         FROM profile_packages WHERE profile_id=?1",
        params![profile_id, package_id],
    )?;
    connection
        .query_row(
            "SELECT priority FROM profile_packages WHERE profile_id=?1 AND package_id=?2",
            params![profile_id, package_id],
            |row| row.get(0),
        )
        .context("appended profile membership is missing")
}

/// Move every membership of `from_package` to `to_package`, keeping priorities
/// and enabled flags. An update uses this so it does not append a new rank.
pub(crate) fn transfer_memberships(
    connection: &Connection,
    from_package: &str,
    to_package: &str,
) -> Result<usize> {
    Ok(connection.execute(
        "UPDATE profile_packages SET package_id=?2 WHERE package_id=?1",
        params![from_package, to_package],
    )?)
}

/// Delete every row that names `package_id`, except the package row itself.
pub(crate) fn delete_package_records(connection: &Connection, package_id: &str) -> Result<()> {
    for statement in [
        "DELETE FROM profile_packages WHERE package_id=?1",
        "DELETE FROM file_claims WHERE package_id=?1",
        "DELETE FROM pending_overrides WHERE package_id=?1",
        "DELETE FROM pending_restores WHERE package_id=?1",
        "DELETE FROM managed_files WHERE package_id=?1",
        "DELETE FROM managed_directories WHERE package_id=?1",
        "DELETE FROM package_conflicts WHERE package_id=?1",
        "DELETE FROM package_requirements WHERE package_id=?1",
        "DELETE FROM pending_stack_packages WHERE package_id=?1",
        "DELETE FROM package_dependencies WHERE package_id=?1",
        "DELETE FROM package_dependencies WHERE dependency_id=?1",
        "DELETE FROM catalog_provenance WHERE package_id=?1",
        "DELETE FROM managed_package_releases WHERE package_id=?1",
    ] {
        connection.execute(statement, [package_id])?;
    }
    Ok(())
}
