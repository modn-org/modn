use super::{record_root, resolve_root, RootKind};
use crate::state::Store;
use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

const V2_FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/state/fixtures/state-v2.sqlite"
);

fn copy_v2_fixture(dir: &Path) -> PathBuf {
    let db = dir.join("state.sqlite");
    fs::copy(V2_FIXTURE, &db).expect("v2 fixture");
    db
}

fn user_version(db: &Path) -> i64 {
    Connection::open(db)
        .expect("db")
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("version")
}

fn rows(connection: &Connection, query: &str) -> Vec<String> {
    let mut statement = connection.prepare(query).expect("query");
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("rows")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("rows")
}

fn ownership_rows(connection: &Connection) -> Vec<String> {
    let mut all = Vec::new();
    for table in [
        "managed_files",
        "managed_directories",
        "created_directories",
    ] {
        all.extend(rows(
            connection,
            &format!(
                "SELECT '{table}|' || game_path || '|' || path_key || '|' || relative_path
                 FROM {table} ORDER BY game_path, path_key"
            ),
        ));
    }
    all
}

#[test]
fn v2_state_migrates_to_v3_with_every_owned_path_on_the_game_root() {
    let temp = tempdir().expect("fixture");
    let db = copy_v2_fixture(temp.path());
    let before = ownership_rows(&Connection::open(&db).expect("v2"));
    let games = rows(
        &Connection::open(&db).expect("v2"),
        "SELECT DISTINCT game_path FROM managed_packages",
    );
    assert_eq!(user_version(&db), 2);
    assert_eq!(
        before.len(),
        6,
        "fixture holds files, directories and created folders"
    );
    assert_eq!(games.len(), 1);

    let store = Store::open(&db).expect("migration");
    assert_eq!(user_version(&db), 4);
    assert_eq!(ownership_rows(&store.connection), before);
    for table in [
        "managed_files",
        "managed_directories",
        "created_directories",
    ] {
        assert_eq!(
            rows(
                &store.connection,
                &format!("SELECT DISTINCT root FROM {table}")
            ),
            ["game"]
        );
    }
    assert_eq!(
        rows(
            &store.connection,
            "SELECT game_path || '|' || root || '|' || path FROM installation_roots"
        ),
        [format!("{0}|game|{0}", games[0])]
    );
    drop(store);

    let migrated = fs::read(&db).expect("migrated bytes");
    drop(Store::open(&db).expect("second open"));
    assert_eq!(fs::read(&db).expect("reopened bytes"), migrated);
}

#[test]
fn failed_v3_migration_leaves_v2_state_intact_and_readable() {
    let temp = tempdir().expect("fixture");
    let db = copy_v2_fixture(temp.path());
    Connection::open(&db)
        .expect("v2")
        .execute_batch("CREATE TABLE created_directories_v3(stray TEXT);")
        .expect("conflicting table");
    let before = fs::read(&db).expect("v2 bytes");

    assert!(Store::open(&db).is_err());
    assert_eq!(fs::read(&db).expect("after bytes"), before);
    assert_eq!(user_version(&db), 2);
    let store = Store::open_readonly(&db).expect("v2 stays readable");
    assert_eq!(ownership_rows(&store.connection).len(), 6);
}

#[cfg(windows)]
#[test]
fn v2_installations_with_one_canonical_root_refuse_migration() {
    let temp = tempdir().expect("fixture");
    let db = copy_v2_fixture(temp.path());
    let connection = Connection::open(&db).expect("v2");
    connection
        .execute_batch(
            "INSERT INTO managed_packages(id,game_path,archive_sha256)
             SELECT 'twin', upper(game_path), archive_sha256 FROM managed_packages LIMIT 1;",
        )
        .expect("case variant installation");
    drop(connection);
    let before = fs::read(&db).expect("v2 bytes");

    let error = Store::open(&db).err().expect("collision refuses migration");
    assert!(format!("{error:#}").contains("two installations cannot share a root"));
    assert_eq!(fs::read(&db).expect("after bytes"), before);
}

#[test]
fn readonly_open_reads_v2_state_through_game_roots_without_writes() {
    let temp = tempdir().expect("fixture");
    let db = copy_v2_fixture(temp.path());
    fs::write(temp.path().join(".schema.lock"), b"").expect("lock");
    let before = fs::read(&db).expect("v2 bytes");

    let store = Store::open_readonly(&db).expect("readonly v2");
    assert_eq!(
        rows(
            &store.connection,
            "SELECT DISTINCT root FROM managed_files UNION SELECT root FROM installation_roots"
        ),
        ["game"]
    );
    drop(store);
    assert_eq!(fs::read(&db).expect("after bytes"), before);
}

#[test]
fn two_installations_cannot_claim_one_canonical_root() {
    let temp = tempdir().expect("fixture");
    let store = Store::open(&temp.path().join("state.sqlite")).expect("state");
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    record_root(&store.connection, &first, RootKind::Game, &first).expect("first root");
    record_root(&store.connection, &first, RootKind::Game, &first).expect("idempotent");

    let error =
        record_root(&store.connection, &second, RootKind::Game, &first).expect_err("shared root");
    assert!(format!("{error:#}").contains("two installations cannot share a root"));
    #[cfg(windows)]
    {
        let folded = PathBuf::from(first.to_string_lossy().to_uppercase());
        assert!(record_root(&store.connection, &second, RootKind::Game, &folded).is_err());
    }
    assert_eq!(
        rows(
            &store.connection,
            "SELECT game_path FROM installation_roots"
        ),
        [first.to_string_lossy().into_owned()]
    );
}

pub(crate) fn link_directory(link: &Path, target: &Path) {
    #[cfg(windows)]
    {
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .expect("mklink")
            .status;
        assert!(status.success());
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).expect("symlink");
}

pub(crate) fn unlink_directory(link: &Path) {
    #[cfg(windows)]
    fs::remove_dir(link).expect("remove junction");
    #[cfg(unix)]
    fs::remove_file(link).expect("remove symlink");
}

#[test]
fn a_moved_or_linked_root_fails_closed() {
    let temp = tempdir().expect("fixture");
    let store = Store::open(&temp.path().join("state/state.sqlite")).expect("state");
    fs::create_dir_all(temp.path().join("a/game")).expect("game");
    let game = temp
        .path()
        .join("a/game")
        .canonicalize()
        .expect("canonical");
    record_root(&store.connection, &game, RootKind::Game, &game).expect("root");
    assert_eq!(
        resolve_root(&store.connection, &game, RootKind::Game).expect("resolves"),
        game
    );

    let parent = game.parent().expect("parent").to_path_buf();
    let moved = parent.with_file_name("b");
    fs::rename(&parent, &moved).expect("move");
    let error = resolve_root(&store.connection, &game, RootKind::Game).expect_err("missing");
    assert!(!format!("{error:#}").is_empty());

    link_directory(&parent, &moved);
    let error = resolve_root(&store.connection, &game, RootKind::Game).expect_err("moved");
    assert!(format!("{error:#}").contains("moved or a parent became a link"));

    unlink_directory(&parent);
    fs::rename(&moved, &parent).expect("restore");
    let real = temp.path().join("real");
    fs::rename(&game, &real).expect("move game");
    link_directory(&game, &real);
    let error = resolve_root(&store.connection, &game, RootKind::Game).expect_err("link");
    assert!(format!("{error:#}").contains("not a link"));
}

const PENDING_ROWS: &str =
    "SELECT 'tx|' || id || '|' || action || '|' || status || '|' || game_path
        FROM pending_transactions
    UNION ALL SELECT 'file|' || ord || '|' || relative_path || '|' || sha256 || '|' || bytes
        FROM pending_files
    UNION ALL SELECT 'dir|' || ord || '|' || relative_path || '|' || identity
        FROM pending_directories
    UNION ALL SELECT 'created|' || ord || '|' || relative_path || '|' || identity
        FROM pending_created_directories
    ORDER BY 1";

const V4_PENDING_COLUMNS: [(&str, &str); 4] = [
    ("pending_files", "root"),
    ("pending_directories", "root"),
    ("pending_created_directories", "root"),
    ("pending_created_directories", "base_path"),
];

fn has_column(connection: &Connection, table: &str, column: &str) -> bool {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name=?2)",
            [table, column],
            |row| row.get(0),
        )
        .expect("table info")
}

fn v3_fixture_with_pending_rows(dir: &Path) -> (PathBuf, String) {
    let db = dir.join("state.sqlite");
    let game = dir.join("game");
    fs::create_dir_all(&game).expect("game");
    let game = game.canonicalize().expect("canonical game");
    let game_text = game.to_string_lossy().into_owned();
    let store = Store::open(&db).expect("state");
    record_root(&store.connection, &game, RootKind::Game, &game).expect("game root");
    store
        .connection
        .execute_batch(&format!(
            "INSERT INTO managed_packages(id,game_path,archive_sha256)
                 VALUES ('kept','{game_text}','aa');
             INSERT INTO managed_files(game_path,root,path_key,relative_path,package_id,sha256,bytes)
                 VALUES ('{game_text}','game','mods/kept.bin','mods/kept.bin','kept','bb',3);
             INSERT INTO pending_transactions(id,action,status,game_path,package_id,archive_sha256)
                 VALUES ('t1','install','prepared','{game_text}','next','cc');
             INSERT INTO pending_files(transaction_id,ord,relative_path,sha256,bytes)
                 VALUES ('t1',0,'mods/a.bin','dd',1), ('t1',1,'mods/b.bin','ee',2);
             INSERT INTO pending_directories(transaction_id,ord,relative_path,identity)
                 VALUES ('t1',0,'mods/own','id-own');
             INSERT INTO pending_created_directories(transaction_id,ord,relative_path,identity)
                 VALUES ('t1',0,'mods','id-mods');"
        ))
        .expect("v3 rows");
    for (table, column) in V4_PENDING_COLUMNS {
        store
            .connection
            .execute_batch(&format!("ALTER TABLE {table} DROP COLUMN {column};"))
            .expect("drop v4 column");
    }
    store
        .connection
        .pragma_update(None, "user_version", 3)
        .expect("v3");
    drop(store);
    (db, game_text)
}

#[test]
fn v3_state_migrates_to_v4_with_pending_rows_on_the_game_root() {
    let temp = tempdir().expect("fixture");
    let (db, game) = v3_fixture_with_pending_rows(temp.path());
    let v3 = Connection::open(&db).expect("v3");
    let pending = rows(&v3, PENDING_ROWS);
    let owned = ownership_rows(&v3);
    assert_eq!(pending.len(), 5);
    assert!(V4_PENDING_COLUMNS
        .iter()
        .all(|(table, column)| !has_column(&v3, table, column)));
    drop(v3);
    assert_eq!(user_version(&db), 3);

    let store = Store::open(&db).expect("migration");
    assert_eq!(user_version(&db), 4);
    assert_eq!(rows(&store.connection, PENDING_ROWS), pending);
    assert_eq!(ownership_rows(&store.connection), owned);
    assert_eq!(
        rows(
            &store.connection,
            "SELECT root FROM pending_files UNION SELECT root FROM pending_directories
             UNION SELECT root || '|' || COALESCE(base_path, 'none')
                 FROM pending_created_directories"
        ),
        ["game", "game|none"]
    );
    assert_eq!(
        rows(
            &store.connection,
            "SELECT game_path || '|' || root || '|' || path FROM installation_roots"
        ),
        [format!("{game}|game|{game}")]
    );
    drop(store);

    let migrated = fs::read(&db).expect("migrated bytes");
    drop(Store::open(&db).expect("second open"));
    assert_eq!(fs::read(&db).expect("reopened bytes"), migrated);
    assert_eq!(user_version(&db), 4);
}

#[test]
fn failed_v4_migration_leaves_v3_state_intact_and_readable() {
    let temp = tempdir().expect("fixture");
    let (db, game) = v3_fixture_with_pending_rows(temp.path());
    Connection::open(&db)
        .expect("v3")
        .execute(
            "UPDATE installation_roots SET game_path='elsewhere' WHERE game_path=?1",
            [&game],
        )
        .expect("conflicting root claim");
    let v3 = Connection::open(&db).expect("v3");
    let pending = rows(&v3, PENDING_ROWS);
    let owned = ownership_rows(&v3);
    drop(v3);
    let before = fs::read(&db).expect("v3 bytes");

    let error = Store::open(&db).err().expect("conflict fails migration");
    assert!(format!("{error:#}").contains("two installations cannot share a root"));
    assert_eq!(fs::read(&db).expect("after bytes"), before);
    assert_eq!(user_version(&db), 3);
    let store = Store::open_readonly(&db).expect("v3 stays readable");
    assert_eq!(
        store.installed(Path::new(&game)).expect("packages").len(),
        1
    );
    assert_eq!(rows(&store.connection, PENDING_ROWS), pending);
    assert_eq!(ownership_rows(&store.connection), owned);
    assert!(V4_PENDING_COLUMNS.iter().all(|(table, column)| !has_column(
        &store.connection,
        table,
        column
    )));
}
