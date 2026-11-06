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
