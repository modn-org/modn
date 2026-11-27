use super::Store;
use rusqlite::{params, Connection};
use std::fs;
use tempfile::tempdir;

fn table_columns(store: &Store, table: &str) -> Vec<(String, i64)> {
    store
        .connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .expect("table info")
        .query_map([], |row| Ok((row.get(1)?, row.get(3)?)))
        .expect("column query")
        .collect::<rusqlite::Result<_>>()
        .expect("column rows")
}

fn column_names(store: &Store, table: &str) -> Vec<String> {
    table_columns(store, table)
        .into_iter()
        .map(|(name, _)| name)
        .collect()
}

fn insert_package(store: &Store, id: &str, game: &str) {
    store
        .connection
        .execute(
            "INSERT INTO managed_packages(id,game_path,archive_blake3) VALUES (?1,?2,'hash')",
            params![id, game],
        )
        .expect("package");
}

fn insert_profile(store: &Store, id: &str, game: &str, name: &str, active: i64) {
    store
        .connection
        .execute(
            "INSERT INTO profiles(id,game_path,name,active) VALUES (?1,?2,?3,?4)",
            params![id, game, name, active],
        )
        .expect("profile");
}

#[test]
fn a_fresh_database_has_schema_version_eight_and_the_profile_layout() {
    let temp = tempdir().expect("fixture");
    let db = temp.path().join("state.sqlite");
    let store = Store::open(&db).expect("state");
    let version: i64 = store
        .connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("version");
    assert_eq!(version, 8);
    assert_eq!(version, crate::state::roots::SCHEMA_VERSION);
    assert_eq!(
        column_names(&store, "profiles"),
        ["id", "game_path", "name", "active", "created_at"]
    );
    assert_eq!(
        column_names(&store, "profile_packages"),
        ["profile_id", "package_id", "priority", "enabled"]
    );
    assert_eq!(
        column_names(&store, "file_claims"),
        [
            "package_id",
            "game_path",
            "root",
            "path_key",
            "relative_path",
            "archive_entry",
            "blake3",
            "bytes"
        ]
    );
    assert_eq!(
        column_names(&store, "pending_overrides"),
        [
            "transaction_id",
            "ord",
            "root",
            "relative_path",
            "package_id",
            "blake3",
            "bytes"
        ]
    );
    assert_eq!(
        column_names(&store, "pending_restores"),
        [
            "transaction_id",
            "ord",
            "package_id",
            "archive_entry",
            "blake3",
            "bytes"
        ]
    );
    assert_eq!(
        column_names(&store, "pending_profile_switches"),
        ["transaction_id", "game_path", "from_profile", "to_profile"]
    );
    assert_eq!(
        column_names(&store, "history_profile_switches"),
        ["operation_id", "game_path", "from_profile", "to_profile"]
    );
    let winner_columns = table_columns(&store, "managed_files");
    assert!(winner_columns.contains(&("profile_id".to_owned(), 1)));
    assert!(winner_columns.contains(&("archive_entry".to_owned(), 1)));
    let index_sql: String = store
        .connection
        .query_row(
            "SELECT sql FROM sqlite_master
             WHERE type='index' AND name='profiles_one_active_per_game'",
            [],
            |row| row.get(0),
        )
        .expect("partial unique index");
    assert!(index_sql.contains("WHERE active = 1"), "{index_sql}");
    let on_delete: String = store
        .connection
        .query_row(
            "SELECT on_delete FROM pragma_foreign_key_list('profile_packages')
             WHERE \"from\"='profile_id'",
            [],
            |row| row.get(0),
        )
        .expect("profile membership foreign key");
    assert_eq!(on_delete, "CASCADE");
}

#[test]
fn profile_names_are_unique_within_one_game_path() {
    let temp = tempdir().expect("fixture");
    let store = Store::open(&temp.path().join("state.sqlite")).expect("state");
    insert_profile(&store, "one", "/game", "default", 0);
    let duplicate = store.connection.execute(
        "INSERT INTO profiles(id,game_path,name,active)
         VALUES ('two','/game','default',1)",
        [],
    );
    assert!(
        duplicate.is_err(),
        "duplicate (game_path, name) was accepted"
    );
    insert_profile(&store, "three", "/other", "default", 1);
}

#[test]
fn one_active_profile_is_allowed_per_game_path() {
    let temp = tempdir().expect("fixture");
    let store = Store::open(&temp.path().join("state.sqlite")).expect("state");
    insert_profile(&store, "one", "/game", "default", 1);
    insert_profile(&store, "spare", "/game", "spare", 0);
    let second = store.connection.execute(
        "INSERT INTO profiles(id,game_path,name,active)
         VALUES ('two','/game','other',1)",
        [],
    );
    assert!(
        second.is_err(),
        "a second active profile was accepted for one game path"
    );
    insert_profile(&store, "other-game", "/other", "default", 1);
    insert_profile(&store, "other-spare", "/other", "spare", 0);
}

#[test]
fn profile_package_priorities_are_unique_and_non_negative() {
    let temp = tempdir().expect("fixture");
    let store = Store::open(&temp.path().join("state.sqlite")).expect("state");
    insert_package(&store, "pkg-one", "/game");
    insert_package(&store, "pkg-two", "/game");
    insert_profile(&store, "profile", "/game", "default", 1);
    store
        .connection
        .execute(
            "INSERT INTO profile_packages(profile_id,package_id,priority)
             VALUES ('profile','pkg-one',0)",
            [],
        )
        .expect("first membership");
    let duplicate_priority = store.connection.execute(
        "INSERT INTO profile_packages(profile_id,package_id,priority)
         VALUES ('profile','pkg-two',0)",
        [],
    );
    assert!(
        duplicate_priority.is_err(),
        "duplicate (profile_id, priority) was accepted"
    );
    let duplicate_package = store.connection.execute(
        "INSERT INTO profile_packages(profile_id,package_id,priority)
         VALUES ('profile','pkg-one',5)",
        [],
    );
    assert!(
        duplicate_package.is_err(),
        "duplicate (profile_id, package_id) was accepted"
    );
    store
        .connection
        .execute(
            "INSERT INTO profile_packages(profile_id,package_id,priority)
             VALUES ('profile','pkg-two',1)",
            [],
        )
        .expect("second membership");
    let negative = store.connection.execute(
        "INSERT INTO profile_packages(profile_id,package_id,priority)
         VALUES ('profile','pkg-two',-1)",
        [],
    );
    assert!(negative.is_err(), "a negative priority was accepted");
    let not_boolean = store.connection.execute(
        "INSERT INTO profile_packages(profile_id,package_id,priority,enabled)
         VALUES ('profile','pkg-two',2)",
        [],
    );
    assert!(not_boolean.is_err(), "enabled outside 0 and 1 was accepted");
}

#[test]
fn deleting_a_profile_cascades_its_memberships_and_keeps_packages_and_claims() {
    let temp = tempdir().expect("fixture");
    let store = Store::open(&temp.path().join("state.sqlite")).expect("state");
    store
        .connection
        .execute_batch("PRAGMA foreign_keys = ON")
        .expect("foreign keys");
    insert_package(&store, "pkg", "/game");
    insert_profile(&store, "profile", "/game", "default", 1);
    store
        .connection
        .execute(
            "INSERT INTO profile_packages(profile_id,package_id,priority)
             VALUES ('profile','pkg',0)",
            [],
        )
        .expect("membership");
    store
        .connection
        .execute(
            "INSERT INTO file_claims
             (package_id,game_path,root,path_key,relative_path,archive_entry,blake3,bytes)
             VALUES ('pkg','/game','game','one.archive','one.archive','one.archive','aa',1)",
            [],
        )
        .expect("claim");
    store
        .connection
        .execute("DELETE FROM profiles WHERE id='profile'", [])
        .expect("delete profile");
    let memberships: i64 = store
        .connection
        .query_row("SELECT COUNT(*) FROM profile_packages", [], |row| {
            row.get(0)
        })
        .expect("membership count");
    assert_eq!(memberships, 0);
    for table in ["managed_packages", "file_claims"] {
        let rows: i64 = store
            .connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("kept rows");
        assert_eq!(rows, 1, "{table} must survive a profile deletion");
    }
}

#[test]
fn earlier_schema_versions_are_refused_without_being_read_or_changed() {
    for version in [6, 7] {
        let temp = tempdir().expect("fixture");
        let db = temp.path().join("state.sqlite");
        let connection = Connection::open(&db).expect("fixture database");
        connection
            .execute_batch(&format!(
                "CREATE TABLE managed_packages (
                    id TEXT PRIMARY KEY, game_path TEXT NOT NULL,
                    archive_blake3 TEXT NOT NULL,
                    pinned INTEGER NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                );
                CREATE TABLE managed_files (
                    game_path TEXT NOT NULL, root TEXT NOT NULL DEFAULT 'game',
                    path_key TEXT NOT NULL, relative_path TEXT NOT NULL,
                    package_id TEXT NOT NULL, blake3 TEXT NOT NULL, bytes INTEGER NOT NULL,
                    PRIMARY KEY(game_path, root, path_key)
                );
                CREATE TABLE pending_files (
                    transaction_id TEXT NOT NULL, ord INTEGER NOT NULL,
                    root TEXT NOT NULL DEFAULT 'game', relative_path TEXT NOT NULL,
                    blake3 TEXT NOT NULL, bytes INTEGER NOT NULL,
                    PRIMARY KEY(transaction_id, ord)
                );
                INSERT INTO managed_packages(id,game_path,archive_blake3)
                VALUES ('kept','/game','aa');
                PRAGMA user_version = {version};"
            ))
            .expect("earlier schema");
        drop(connection);
        fs::write(temp.path().join(".schema.lock"), b"").expect("lock");
        let before = fs::read(&db).expect("database bytes");
        let expected = format!(
            "state database {} has schema version {version}, but this Modn build needs schema version {}",
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
            assert!(message.contains("did not read or change"), "{message}");
        }
        assert_eq!(fs::read(&db).expect("database bytes"), before);
    }
}

#[test]
fn a_version_eight_database_missing_a_profile_table_is_refused() {
    for table in crate::state::profiles::TABLES {
        let temp = tempdir().expect("fixture");
        let db = temp.path().join("state.sqlite");
        let store = Store::open(&db).expect("state");
        store
            .connection
            .execute_batch(&format!("DROP TABLE {table}"))
            .expect("drop table");
        drop(store);
        let before = fs::read(&db).expect("database bytes");
        for error in [
            Store::open(&db).err().expect("read-write open must refuse"),
            Store::open_readonly(&db)
                .err()
                .expect("read-only open must refuse"),
        ] {
            let message = format!("{error:#}");
            assert!(
                message.contains("It does not use the table layout this build writes"),
                "{table}: {message}"
            );
            assert!(message.contains("schema version 8"), "{table}: {message}");
        }
        assert_eq!(fs::read(&db).expect("database bytes"), before, "{table}");
    }
}

#[test]
fn a_version_eight_database_without_the_winner_profile_columns_is_refused() {
    let temp = tempdir().expect("fixture");
    let db = temp.path().join("state.sqlite");
    let store = Store::open(&db).expect("state");
    store
        .connection
        .execute_batch(
            "DROP TABLE managed_files;
             CREATE TABLE managed_files (
                 game_path TEXT NOT NULL,
                 root TEXT NOT NULL DEFAULT 'game',
                 path_key TEXT NOT NULL,
                 relative_path TEXT NOT NULL,
                 package_id TEXT NOT NULL REFERENCES managed_packages(id),
                 blake3 TEXT NOT NULL,
                 bytes INTEGER NOT NULL,
                 PRIMARY KEY(game_path, root, path_key)
             );",
        )
        .expect("older winner layout");
    drop(store);
    let before = fs::read(&db).expect("database bytes");
    for error in [
        Store::open(&db).err().expect("read-write open must refuse"),
        Store::open_readonly(&db)
            .err()
            .expect("read-only open must refuse"),
    ] {
        let message = format!("{error:#}");
        assert!(
            message.contains("It does not use the table layout this build writes"),
            "{message}"
        );
        assert!(message.contains("schema version 8"), "{message}");
    }
    assert_eq!(fs::read(&db).expect("database bytes"), before);
}
