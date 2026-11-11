use super::{ensure_schema, Capabilities, SCHEMA_VERSION};
use crate::state::Store;
use rusqlite::Connection;
use std::path::Path;
use tempfile::tempdir;

fn game() -> &'static Path {
    Path::new("C:/Games/Example")
}

#[test]
fn record_and_widen_keep_every_approved_capability() {
    let temp = tempdir().expect("fixture");
    let db = temp.path().join("state.sqlite");
    let mut store = Store::open(&db).expect("state");
    assert_eq!(store.approval(game(), "mod").expect("approval"), None);

    let first = Capabilities::new(true, ["engine/tools/first.exe", "engine/tools/second.exe"]);
    store
        .record_approval(game(), "mod", &first)
        .expect("record");
    assert_eq!(
        store.approval(game(), "mod").expect("approval"),
        Some(first.clone())
    );

    let later = Capabilities::new(false, ["engine/tools/third.exe"]);
    store.record_approval(game(), "mod", &later).expect("widen");
    let stored = store
        .approval(game(), "mod")
        .expect("approval")
        .expect("row");
    assert!(
        stored.native_injector,
        "a dropped capability stays approved"
    );
    assert_eq!(
        stored.executables,
        first
            .executables
            .union(&later.executables)
            .cloned()
            .collect()
    );
}

#[test]
fn approvals_are_scoped_to_one_game_installation_and_mod() {
    let temp = tempdir().expect("fixture");
    let db = temp.path().join("state.sqlite");
    let mut store = Store::open(&db).expect("state");
    store
        .record_approval(
            game(),
            "mod",
            &Capabilities::new(false, ["engine/tools/a.exe"]),
        )
        .expect("record");
    assert!(store
        .approval(Path::new("C:/Games/Other"), "mod")
        .expect("other game")
        .is_none());
    assert!(store
        .approval(game(), "other-mod")
        .expect("other mod")
        .is_none());
}

#[test]
fn schema_step_records_version_five_and_reopening_keeps_approvals() {
    let temp = tempdir().expect("fixture");
    let db = temp.path().join("state.sqlite");
    let mut store = Store::open(&db).expect("state");
    store
        .record_approval(
            game(),
            "mod",
            &Capabilities::new(true, Vec::<String>::new()),
        )
        .expect("record");
    let version: i64 = store
        .connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("version");
    assert_eq!(version, SCHEMA_VERSION);
    drop(store);

    let reopened = Store::open_readonly(&db).expect("read-only state");
    assert_eq!(
        reopened
            .approval(game(), "mod")
            .expect("approval")
            .map(|stored| stored.native_injector),
        Some(true)
    );
}

#[test]
fn read_only_v4_state_reports_no_approval_without_writing() {
    let temp = tempdir().expect("fixture");
    let db = temp.path().join("state.sqlite");
    let store = Store::open(&db).expect("state");
    drop(store);
    let connection = Connection::open(&db).expect("database");
    connection
        .execute_batch(
            "DROP TABLE mod_approvals;
             PRAGMA user_version = 4;",
        )
        .expect("downgrade fixture");
    drop(connection);

    let readonly = Store::open_readonly(&db).expect("v4 read-only state");
    assert!(readonly
        .approval(game(), "mod")
        .expect("approval")
        .is_none());
    assert_eq!(
        Connection::open(&db)
            .expect("reopen")
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .expect("version"),
        4,
        "a read-only open must not migrate"
    );

    let mut writable = Store::open(&db).expect("migrated state");
    ensure_schema(&writable.connection).expect("idempotent schema step");
    writable
        .record_approval(
            game(),
            "mod",
            &Capabilities::new(false, Vec::<String>::new()),
        )
        .expect("record after migration");
}
