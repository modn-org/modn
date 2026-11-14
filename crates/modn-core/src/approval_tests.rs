use super::Capabilities;
use crate::state::Store;
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
    assert_eq!(
        store.approval(game(), "main", "mod").expect("approval"),
        None
    );

    let first = Capabilities::new(true, ["engine/tools/first.exe", "engine/tools/second.exe"]);
    store
        .record_approval(game(), "main", "mod", &first)
        .expect("record");
    assert_eq!(
        store.approval(game(), "main", "mod").expect("approval"),
        Some(first.clone())
    );

    let later = Capabilities::new(false, ["engine/tools/third.exe"]);
    store
        .record_approval(game(), "main", "mod", &later)
        .expect("widen");
    let stored = store
        .approval(game(), "main", "mod")
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
fn approvals_are_scoped_to_one_game_installation_bucket_and_mod() {
    let temp = tempdir().expect("fixture");
    let db = temp.path().join("state.sqlite");
    let mut store = Store::open(&db).expect("state");
    store
        .record_approval(
            game(),
            "main",
            "mod",
            &Capabilities::new(false, ["engine/tools/a.exe"]),
        )
        .expect("record");
    assert!(store
        .approval(Path::new("C:/Games/Other"), "main", "mod")
        .expect("other game")
        .is_none());
    assert!(store
        .approval(game(), "main", "other-mod")
        .expect("other mod")
        .is_none());
    assert!(store
        .approval(game(), "other-bucket", "mod")
        .expect("other bucket")
        .is_none());
}

#[test]
fn schema_step_records_the_current_version_and_reopening_keeps_approvals() {
    let temp = tempdir().expect("fixture");
    let db = temp.path().join("state.sqlite");
    let mut store = Store::open(&db).expect("state");
    store
        .record_approval(
            game(),
            "main",
            "mod",
            &Capabilities::new(true, Vec::<String>::new()),
        )
        .expect("record");
    let version: i64 = store
        .connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("version");
    assert_eq!(version, 6);
    drop(store);

    let reopened = Store::open_readonly(&db).expect("read-only state");
    assert_eq!(
        reopened
            .approval(game(), "main", "mod")
            .expect("approval")
            .map(|stored| stored.native_injector),
        Some(true)
    );
}
