use super::Store;
use rusqlite::Connection;
use std::fs;
use tempfile::tempdir;

#[test]
fn fresh_packages_require_archive_provenance() {
    let temp = tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let game = temp.path().join("game");
    fs::create_dir(&game).unwrap();
    let store = Store::open(&db).unwrap();
    let columns: Vec<(String, i64)> = store
        .connection
        .prepare("PRAGMA table_info(managed_packages)")
        .unwrap()
        .query_map([], |row| Ok((row.get(1)?, row.get(3)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert!(columns.contains(&("archive_sha256".into(), 1)));
    assert!(!columns.iter().any(|(name, _)| name == "origin"));
    assert!(store
        .connection
        .execute(
            "INSERT INTO managed_packages(id,game_path,archive_sha256)
         VALUES ('manual','/game',NULL)",
            [],
        )
        .is_err());
    store
        .connection
        .execute(
            "INSERT INTO managed_packages(id,game_path,archive_sha256)
         VALUES ('installed',?1,?2)",
            rusqlite::params![
                game.canonicalize().unwrap().to_string_lossy().as_ref(),
                "a".repeat(64)
            ],
        )
        .unwrap();
    assert_eq!(
        store.installed(&game).unwrap()[0].archive_sha256,
        "a".repeat(64)
    );
    let original = fs::read(&db).unwrap();
    drop(store);
    let readonly = Store::open_readonly(&db).unwrap();
    assert_eq!(readonly.installed(&game).unwrap().len(), 1);
    drop(readonly);
    assert_eq!(fs::read(&db).unwrap(), original);
}

#[test]
fn old_archive_origin_schema_is_rejected_even_with_required_digest() {
    let temp = tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE managed_packages (
            id TEXT PRIMARY KEY, game_path TEXT NOT NULL,
            archive_sha256 TEXT NOT NULL,
            origin TEXT NOT NULL DEFAULT 'archive',
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );",
    )
    .unwrap();
    drop(conn);
    assert!(Store::open_readonly(&db).is_err());
    assert!(Store::open(&db).is_err());
}

#[test]
fn old_nullable_archive_schema_is_rejected_without_origin() {
    let temp = tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE managed_packages (
            id TEXT PRIMARY KEY, game_path TEXT NOT NULL,
            archive_sha256 TEXT,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );",
    )
    .unwrap();
    drop(conn);
    assert!(Store::open_readonly(&db).is_err());
    assert!(Store::open(&db).is_err());
}

#[test]
fn old_manual_ownership_schema_is_rejected_not_migrated() {
    let temp = tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE managed_packages (
            id TEXT PRIMARY KEY, game_path TEXT NOT NULL,
            archive_sha256 TEXT,
            origin TEXT NOT NULL DEFAULT 'archive',
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        INSERT INTO managed_packages(id,game_path,archive_sha256,origin)
        VALUES ('manual','/game',NULL,'manual');",
    )
    .unwrap();
    drop(conn);
    assert!(Store::open_readonly(&db).is_err());
    assert!(Store::open(&db).is_err());
    let conn = Connection::open(&db).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT origin FROM managed_packages WHERE id='manual'",
            [],
            |row| { row.get::<_, String>(0) }
        )
        .unwrap(),
        "manual"
    );
}
