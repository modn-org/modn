//! Profile, file-claim and profile-work tables of the state database.
//!
//! `Store::open` runs [`DDL`] in the fresh-database schema batch, after the
//! ownership tables. There is no migration: an existing database at an earlier
//! schema version is refused before any of these tables is read. Profile
//! lifecycle, claim and switch SQL belongs in this module.

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
