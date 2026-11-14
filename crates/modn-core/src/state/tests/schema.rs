use super::Store;
use rusqlite::Connection;
use std::fs;
use tempfile::tempdir;

#[test]
fn fresh_state_uses_archive_blake3_in_every_table() {
    let temp = tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let store = Store::open(&db).unwrap();
    for table in [
        "managed_packages",
        "pending_transactions",
        "package_conflicts",
        "package_requirements",
    ] {
        let columns: Vec<String> = store
            .connection
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap()
            .query_map([], |row| row.get(1))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(
            columns.iter().any(|column| column == "archive_blake3"),
            "{table} has no archive_blake3 column: {columns:?}"
        );
        assert!(
            !columns.iter().any(|column| column == "archive_sha256"),
            "{table} still has archive_sha256: {columns:?}"
        );
    }
    for table in ["managed_files", "pending_files"] {
        let columns: Vec<String> = store
            .connection
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap()
            .query_map([], |row| row.get(1))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(
            columns.iter().any(|column| column == "blake3"),
            "{table} has no blake3 column: {columns:?}"
        );
        assert!(
            !columns.iter().any(|column| column == "sha256"),
            "{table} still has a sha256 column: {columns:?}"
        );
    }
}

#[test]
fn an_earlier_schema_database_is_refused_without_being_read_or_changed() {
    let temp = tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let connection = Connection::open(&db).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE managed_packages (
                id TEXT PRIMARY KEY, game_path TEXT NOT NULL,
                archive_blake3 TEXT NOT NULL,
                pinned INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE managed_files (
                game_path TEXT NOT NULL, root TEXT NOT NULL DEFAULT 'game',
                path_key TEXT NOT NULL, relative_path TEXT NOT NULL,
                package_id TEXT NOT NULL, sha256 TEXT NOT NULL, bytes INTEGER NOT NULL,
                PRIMARY KEY(game_path, root, path_key)
            );
            CREATE TABLE pending_transactions (
                id TEXT PRIMARY KEY, action TEXT NOT NULL, status TEXT NOT NULL,
                game_path TEXT NOT NULL, package_id TEXT NOT NULL,
                archive_blake3 TEXT
            );
            CREATE TABLE pending_files (
                transaction_id TEXT NOT NULL, ord INTEGER NOT NULL,
                root TEXT NOT NULL DEFAULT 'game', relative_path TEXT NOT NULL,
                sha256 TEXT NOT NULL, bytes INTEGER NOT NULL,
                PRIMARY KEY(transaction_id, ord)
            );
            INSERT INTO managed_packages(id,game_path,archive_blake3)
            VALUES ('kept','/game','aa');
            INSERT INTO managed_files(game_path,path_key,relative_path,package_id,sha256,bytes)
            VALUES ('/game','owned.archive','owned.archive','kept','bb',1);
            PRAGMA user_version = 6;",
        )
        .unwrap();
    drop(connection);
    fs::write(temp.path().join(".schema.lock"), b"").unwrap();
    let before_bytes = fs::read(&db).unwrap();
    let before_hash = blake3::hash(&before_bytes).to_hex().to_string();
    let before_listing = directory_listing(temp.path());

    let required = crate::state::roots::SCHEMA_VERSION;
    let expected = format!(
        "state database {} has schema version 6, but this Modn build needs schema version {required}",
        db.display()
    );
    for error in [
        Store::open(&db).err().expect("read-write open must refuse"),
        Store::open_readonly(&db)
            .err()
            .expect("read-only open must refuse"),
    ] {
        let message = format!("{error:#}");
        assert!(message.contains(&expected), "{message}");
        assert!(message.contains("did not read or change"), "{message}");
        assert!(message.contains("Move it aside"), "{message}");
        assert!(message.contains("stay on disk"), "{message}");
    }
    assert_eq!(
        blake3::hash(&fs::read(&db).unwrap()).to_hex().to_string(),
        before_hash
    );
    assert_eq!(directory_listing(temp.path()), before_listing);
    assert!(
        !before_listing.iter().any(|name| name == "state.sqlite-wal"
            || name == "state.sqlite-journal"
            || name.contains("backup")),
        "{before_listing:?}"
    );
}

#[test]
fn a_current_version_database_with_the_retired_per_file_column_is_refused() {
    let temp = tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let connection = Connection::open(&db).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE managed_packages (
                id TEXT PRIMARY KEY, game_path TEXT NOT NULL,
                archive_blake3 TEXT NOT NULL,
                pinned INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE managed_files (
                game_path TEXT NOT NULL, root TEXT NOT NULL DEFAULT 'game',
                path_key TEXT NOT NULL, relative_path TEXT NOT NULL,
                package_id TEXT NOT NULL, sha256 TEXT NOT NULL, bytes INTEGER NOT NULL,
                PRIMARY KEY(game_path, root, path_key)
            );
            CREATE TABLE pending_files (
                transaction_id TEXT NOT NULL, ord INTEGER NOT NULL,
                root TEXT NOT NULL DEFAULT 'game', relative_path TEXT NOT NULL,
                sha256 TEXT NOT NULL, bytes INTEGER NOT NULL,
                PRIMARY KEY(transaction_id, ord)
            );
            PRAGMA user_version = 7;",
        )
        .unwrap();
    drop(connection);
    fs::write(temp.path().join(".schema.lock"), b"").unwrap();
    let before = fs::read(&db).unwrap();
    let expected = format!(
        "state database {} has schema version 7, but this Modn build needs schema version {}",
        db.display(),
        crate::state::roots::SCHEMA_VERSION
    );
    for error in [
        Store::open(&db).err().expect("read-write open must refuse"),
        Store::open_readonly(&db)
            .err()
            .expect("read-only open must refuse"),
    ] {
        let message = format!("{error:#}");
        assert!(message.contains(&expected), "{message}");
        assert!(
            message.contains("does not use the table layout this build writes"),
            "{message}"
        );
        assert!(message.contains("did not read or change"), "{message}");
    }
    assert_eq!(fs::read(&db).unwrap(), before);
}

fn directory_listing(directory: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}
