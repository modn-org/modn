use crate::state::Store;
use std::fs;
use tempfile::tempdir;

#[test]
fn version_one_migrates_all_packages_to_unpinned() {
    let temp = tempdir().expect("fixture");
    let db = temp.path().join("state.sqlite");
    let game = temp.path().join("game");
    fs::create_dir(&game).expect("game");
    let store = Store::open(&db).expect("state");
    store
        .connection
        .execute_batch(
            "ALTER TABLE managed_packages DROP COLUMN pinned;
        PRAGMA user_version=1;
        INSERT INTO managed_packages(id,game_path,archive_sha256) VALUES
        ('one','game','hash-one'),('two','game','hash-two');",
        )
        .expect("version one fixture");
    store
        .connection
        .execute(
            "UPDATE managed_packages SET game_path=?1",
            [game
                .canonicalize()
                .expect("game")
                .to_string_lossy()
                .as_ref()],
        )
        .expect("game paths");
    drop(store);
    let before = fs::read(&db).expect("database");
    let readonly = Store::open_readonly(&db).expect("read old schema");
    let old = readonly.installed(&game).expect("old packages");
    assert_eq!(old.len(), 2);
    assert!(old.iter().all(|package| !package.pinned));
    drop(readonly);
    assert_eq!(fs::read(&db).expect("database"), before);
    let store = Store::open(&db).expect("migration");
    let version: i64 = store
        .connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("version");
    assert_eq!(version, 2);
    let packages = store.installed(&game).expect("migrated packages");
    assert_eq!(packages.len(), 2);
    assert!(packages.iter().all(|package| !package.pinned));
    assert_eq!(
        (&packages[0].id, &packages[0].archive_sha256),
        (&"one".to_owned(), &"hash-one".to_owned())
    );
    assert_eq!(
        (&packages[1].id, &packages[1].archive_sha256),
        (&"two".to_owned(), &"hash-two".to_owned())
    );
}
