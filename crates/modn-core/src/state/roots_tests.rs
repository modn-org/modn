use super::{record_root, resolve_root, RootKind};
use crate::state::Store;
use rusqlite::Connection;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

fn rows(connection: &Connection, query: &str) -> Vec<String> {
    let mut statement = connection.prepare(query).expect("query");
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("rows")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("rows")
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
        let folded = std::path::PathBuf::from(first.to_string_lossy().to_uppercase());
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
