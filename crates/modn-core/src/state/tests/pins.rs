use crate::state::Store;
use std::fs;
use tempfile::tempdir;

#[test]
fn fresh_packages_are_unpinned_and_keep_their_archive_pin() {
    let temp = tempdir().expect("fixture");
    let db = temp.path().join("state.sqlite");
    let game = temp.path().join("game");
    fs::create_dir(&game).expect("game");
    let game = game.canonicalize().expect("game");
    let store = Store::open(&db).expect("state");
    for (id, pin) in [("one", "hash-one"), ("two", "hash-two")] {
        store
            .connection
            .execute(
                "INSERT INTO managed_packages(id,game_path,archive_blake3) VALUES (?1,?2,?3)",
                rusqlite::params![id, game.to_string_lossy().as_ref(), pin],
            )
            .expect("package");
    }
    let packages = store.installed(&game).expect("packages");
    assert_eq!(packages.len(), 2);
    assert!(packages.iter().all(|package| !package.pinned));
    assert_eq!(
        (&packages[0].id, &packages[0].archive_blake3),
        (&"one".to_owned(), &"hash-one".to_owned())
    );
    assert_eq!(
        (&packages[1].id, &packages[1].archive_blake3),
        (&"two".to_owned(), &"hash-two".to_owned())
    );
}
