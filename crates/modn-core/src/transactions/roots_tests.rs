use super::generic_tests::{make_zip, plan};
use super::install::Fault;
use super::journal::lock_store;
use crate::state::{link_directory, unlink_directory, Store};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};

struct Lab {
    dir: TempDir,
    store: Store,
    game: PathBuf,
}

impl Lab {
    fn new() -> Self {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("library/game")).unwrap();
        let game = dir.path().join("library/game").canonicalize().unwrap();
        let store = Store::open(&dir.path().join("state/state.sqlite")).unwrap();
        Self { dir, store, game }
    }

    fn archive(&self, name: &str) -> PathBuf {
        let path = self.dir.path().join(format!("{name}.zip"));
        make_zip(
            &path,
            &[
                (&format!("pack/{name}-1.bin"), b"one"),
                (&format!("pack/{name}-2.bin"), b"two"),
            ],
        );
        path
    }

    fn crash_install(&mut self, archive: &Path) {
        let _guard = lock_store(&self.store).unwrap();
        let proposal = plan(&self.game, archive);
        assert!(self
            .store
            .install_locked(
                &self.game,
                archive,
                &proposal,
                Some(Fault::CrashAfter(1)),
                &[],
                None
            )
            .is_err());
        assert_eq!(self.store.pending().unwrap().len(), 1);
    }

    fn state_bytes(&self) -> Vec<u8> {
        fs::read(self.dir.path().join("state/state.sqlite")).unwrap()
    }
}

fn tree(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut entries = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        for entry in fs::read_dir(&path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path.clone());
                entries.push((path, Vec::new()));
            } else {
                entries.push((path.clone(), fs::read(&path).unwrap()));
            }
        }
    }
    entries.sort();
    entries
}

fn schema_version(store: &Store) -> i64 {
    store
        .connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap()
}

#[test]
fn interrupted_install_under_v3_recovers() {
    let mut lab = Lab::new();
    assert_eq!(schema_version(&lab.store), 3);
    let archive = lab.archive("crash");
    lab.crash_install(&archive);
    assert!(lab.game.join("data/mods/crash-1.bin").exists());

    assert_eq!(lab.store.recover().unwrap(), 1);
    assert!(lab.store.pending().unwrap().is_empty());
    assert!(lab.store.installed(&lab.game).unwrap().is_empty());
    assert!(tree(&lab.game).is_empty());

    let package = lab
        .store
        .install_operations_zip(&lab.game, &archive, &plan(&lab.game, &archive))
        .unwrap();
    assert_eq!(package.files.len(), 2);
}

#[test]
fn recovery_refuses_a_moved_root_without_writes() {
    let mut lab = Lab::new();
    let first = lab.archive("first");
    let package = lab
        .store
        .install_operations_zip(&lab.game, &first, &plan(&lab.game, &first))
        .unwrap();
    lab.crash_install(&lab.archive("second"));

    let library = lab.dir.path().join("library");
    let moved = lab.dir.path().join("moved");
    fs::rename(&library, &moved).unwrap();
    link_directory(&library, &moved);
    let state = lab.state_bytes();
    let files = tree(&moved);

    let error = lab.store.recover().unwrap_err();
    assert!(format!("{error:#}").contains("moved or a parent became a link"));
    assert_eq!(lab.state_bytes(), state);
    assert_eq!(tree(&moved), files);

    unlink_directory(&library);
    fs::rename(&moved, &library).unwrap();
    assert_eq!(lab.store.recover().unwrap(), 1);
    assert_eq!(lab.store.installed(&lab.game).unwrap()[0].id, package.id);
}

#[test]
fn uninstall_and_install_refuse_a_moved_root_without_writes() {
    let mut lab = Lab::new();
    let first = lab.archive("first");
    let package = lab
        .store
        .install_operations_zip(&lab.game, &first, &plan(&lab.game, &first))
        .unwrap();
    let library = lab.dir.path().join("library");
    let moved = lab.dir.path().join("moved");
    fs::rename(&library, &moved).unwrap();
    link_directory(&library, &moved);
    let second = lab.archive("second");
    let state = lab.state_bytes();
    let files = tree(&moved);

    let error = lab.store.uninstall(&lab.game, &package.id).unwrap_err();
    assert!(format!("{error:#}").contains("moved or a parent became a link"));
    let error = lab
        .store
        .install_operations_zip(&lab.game, &second, &plan(&lab.game, &second))
        .unwrap_err();
    assert!(format!("{error:#}").contains("moved or a parent became a link"));
    assert_eq!(lab.state_bytes(), state);
    assert_eq!(tree(&moved), files);
}

#[test]
fn recovery_refuses_a_root_replaced_by_a_link_without_writes() {
    let mut lab = Lab::new();
    lab.crash_install(&lab.archive("crash"));
    let real = lab.dir.path().join("real-game");
    fs::rename(&lab.game, &real).unwrap();
    link_directory(&lab.game, &real);
    let state = lab.state_bytes();
    let files = tree(&real);

    let error = lab.store.recover().unwrap_err();
    assert!(format!("{error:#}").contains("not a link"));
    assert_eq!(lab.state_bytes(), state);
    assert_eq!(tree(&real), files);
}
