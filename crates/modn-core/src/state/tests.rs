use super::Store;
use rusqlite::{params, Connection};
use std::fs;
use std::path::Path;
use std::sync::{Arc, Barrier};
use tempfile::tempdir;

fn legacy_ledger(db: &Path, game: &Path) {
    let connection = Connection::open(db).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE managed_packages (
            id TEXT PRIMARY KEY, game_path TEXT NOT NULL,
            archive_sha256 TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE TABLE managed_files (
            game_path TEXT NOT NULL, path_key TEXT NOT NULL,
            relative_path TEXT NOT NULL, package_id TEXT NOT NULL REFERENCES managed_packages(id),
            sha256 TEXT NOT NULL, bytes INTEGER NOT NULL,
            PRIMARY KEY (game_path, path_key)
        );",
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO managed_packages (id, game_path, archive_sha256)
         VALUES ('legacy', ?1, 'original-digest')",
            params![game.to_string_lossy().as_ref()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO managed_files (game_path,path_key,relative_path,package_id,sha256,bytes)
         VALUES (?1,'archive/pc/mod/Legacy.archive','archive/pc/mod/Legacy.archive',
                 'legacy','old-digest',3)",
            params![game.to_string_lossy().as_ref()],
        )
        .unwrap();
}

#[test]
fn legacy_readonly_state_access_does_not_migrate_or_modify_database() {
    let temp = tempdir().unwrap();
    let game = temp.path().join("game");
    let db = temp.path().join("state.sqlite");
    fs::create_dir_all(game.join("bin/x64")).unwrap();
    fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"game").unwrap();
    let mod_path = game.join("archive/pc/mod/Manual.archive");
    fs::create_dir_all(mod_path.parent().unwrap()).unwrap();
    fs::write(&mod_path, b"manual content").unwrap();
    legacy_ledger(&db, &game.canonicalize().unwrap());
    let before = fs::read(&db).unwrap();
    let store = Store::open_readonly(&db).unwrap();
    let installed = store.installed(&game).unwrap();
    assert_eq!(installed.len(), 1);
    assert_eq!(installed[0].files.len(), 1);
    assert_eq!(installed[0].origin, "archive");
    assert_eq!(
        installed[0].archive_sha256.as_deref(),
        Some("original-digest")
    );
    drop(store);
    assert_eq!(
        fs::read(&db).unwrap(),
        before,
        "preview changed legacy state"
    );
}

#[test]
fn simultaneous_legacy_opens_migrate_once_without_errors() {
    let temp = tempdir().unwrap();
    let game = temp.path().join("game");
    let db = temp.path().join("state.sqlite");
    legacy_ledger(&db, &game);
    const WORKERS: usize = 12;
    let barrier = Arc::new(Barrier::new(WORKERS));
    std::thread::scope(|scope| {
        let threads: Vec<_> = (0..WORKERS)
            .map(|_| {
                let barrier = Arc::clone(&barrier);
                let db = db.clone();
                scope.spawn(move || {
                    barrier.wait();
                    let store = Store::open(&db).unwrap();
                    let origin: String = store
                        .connection
                        .query_row(
                            "SELECT origin FROM managed_packages WHERE id='legacy'",
                            [],
                            |row| row.get(0),
                        )
                        .unwrap();
                    assert_eq!(origin, "archive");
                })
            })
            .collect();
        for handle in threads {
            handle.join().unwrap();
        }
    });
    let store = Store::open(&db).unwrap();
    let nullable: i64 = store
        .connection
        .query_row(
            "SELECT \"notnull\" FROM pragma_table_info('managed_packages')
         WHERE name='archive_sha256'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(nullable, 0);
    let count: i64 = store
        .connection
        .query_row("SELECT COUNT(*) FROM managed_packages", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn nullable_migration_preserves_existing_manual_origin() {
    let temp = tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE managed_packages (
            id TEXT PRIMARY KEY, game_path TEXT NOT NULL,
            archive_sha256 TEXT NOT NULL,
            origin TEXT NOT NULL DEFAULT 'archive',
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        INSERT INTO managed_packages (id,game_path,archive_sha256,origin)
        VALUES ('manual','/game','temporary-digest','manual');",
    )
    .unwrap();
    drop(conn);
    let store = Store::open(&db).unwrap();
    let origin: String = store
        .connection
        .query_row(
            "SELECT origin FROM managed_packages WHERE id='manual'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(origin, "manual");
}

#[test]
fn legacy_manual_record_can_only_uninstall_unchanged_files() {
    use sha2::{Digest, Sha256};

    let temp = tempdir().unwrap();
    let game = temp.path().join("game");
    let file = game.join("archive/pc/mod/Legacy.archive");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, b"original").unwrap();
    let game = game.canonicalize().unwrap();
    let mut store = Store::open(&temp.path().join("state/state.sqlite")).unwrap();
    let digest = format!("{:x}", Sha256::digest(b"original"));
    let game_path = game.to_string_lossy();
    store
        .connection
        .execute(
            "INSERT INTO managed_packages(id,game_path,archive_sha256,origin)
             VALUES ('legacy-manual',?1,NULL,'manual')",
            params![game_path.as_ref()],
        )
        .unwrap();
    store
        .connection
        .execute(
            "INSERT INTO managed_files(game_path,path_key,relative_path,package_id,sha256,bytes)
             VALUES (?1,'archive/pc/mod/legacy.archive','archive/pc/mod/Legacy.archive',
                     'legacy-manual',?2,8)",
            params![game_path.as_ref(), digest],
        )
        .unwrap();
    fs::write(&file, b"modified").unwrap();
    assert!(store.uninstall(&game, "legacy-manual").is_err());
    assert_eq!(fs::read(&file).unwrap(), b"modified");
    assert_eq!(store.installed(&game).unwrap().len(), 1);
    assert_eq!(store.recover().unwrap(), 0);
    fs::write(&file, b"original").unwrap();
    store.uninstall(&game, "legacy-manual").unwrap();
    assert!(!file.exists());
    assert!(store.installed(&game).unwrap().is_empty());
}
