use super::catalog_tests::{module_source, plan_into, Fixture};
use super::ManagedPackage;
use crate::game_modules::with_module_source;
use std::fs;
use std::path::Path;

impl Fixture {
    fn install_into(&mut self, archive: &Path, directory: &str) -> anyhow::Result<ManagedPackage> {
        let proposal = plan_into(&self.game, archive, directory);
        let game = self.game.clone();
        with_module_source(Some(&module_source()), || {
            self.store.install_operations_zip(&game, archive, &proposal)
        })
    }

    fn interrupted_into(&mut self, archive: &Path, directory: &str) -> anyhow::Error {
        let proposal = plan_into(&self.game, archive, directory);
        let game = self.game.clone();
        with_module_source(Some(&module_source()), || {
            self.store
                .install_operations_inner(&game, archive, &proposal, Some(1))
        })
        .unwrap_err()
    }

    fn recover(&mut self) {
        with_module_source(Some(&module_source()), || self.store.recover()).unwrap();
    }
}

#[test]
fn uninstall_removes_the_directories_its_install_created() {
    let mut fixture = Fixture::new();
    let archive = fixture.archive("deep");
    let package = fixture.install_into(&archive, "mods/nested/deep").unwrap();
    assert!(fixture.game.join("mods/nested/deep/deep.plugin").is_file());
    assert_eq!(fixture.count("created_directories"), 3);
    assert_eq!(fixture.count("pending_created_directories"), 0);

    fixture.uninstall(&package.id).unwrap();
    assert!(!fixture.game.join("mods").exists());
    assert!(fixture.game.join("bin/TestGame.exe").is_file());
    assert_eq!(fixture.count("created_directories"), 0);
}

#[test]
fn a_directory_that_existed_before_modn_is_never_removed() {
    let mut fixture = Fixture::new();
    fs::create_dir_all(fixture.game.join("mods")).unwrap();
    let archive = fixture.archive("inner");
    let package = fixture.install_into(&archive, "mods/inner").unwrap();
    assert_eq!(fixture.count("created_directories"), 1);
    fixture.uninstall(&package.id).unwrap();
    assert!(!fixture.game.join("mods/inner").exists());
    assert!(fixture.game.join("mods").is_dir());
}

#[test]
fn a_shared_directory_stays_until_its_last_file_is_gone() {
    let mut fixture = Fixture::new();
    let first = fixture.archive("first");
    let second = fixture.archive("second");
    let a = fixture.install_into(&first, "mods/shared").unwrap();
    let b = fixture.install_into(&second, "mods/shared").unwrap();
    assert_eq!(fixture.count("created_directories"), 2);

    fixture.uninstall(&a.id).unwrap();
    assert!(fixture.game.join("mods/shared/second.plugin").is_file());
    assert_eq!(fixture.count("created_directories"), 2);
    fixture.uninstall(&b.id).unwrap();
    assert!(!fixture.game.join("mods").exists());
    assert_eq!(fixture.count("created_directories"), 0);
}

#[test]
fn user_content_keeps_a_directory_until_recovery_finds_it_empty() {
    let mut fixture = Fixture::new();
    let archive = fixture.archive("owned");
    let package = fixture.install_into(&archive, "mods").unwrap();
    let user_file = fixture.game.join("mods/user.txt");
    fs::write(&user_file, b"user").unwrap();

    fixture.uninstall(&package.id).unwrap();
    assert_eq!(fs::read(&user_file).unwrap(), b"user");
    assert_eq!(fixture.count("created_directories"), 1);

    fs::remove_file(&user_file).unwrap();
    fixture.recover();
    assert!(!fixture.game.join("mods").exists());
    assert_eq!(fixture.count("created_directories"), 0);
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn a_replaced_directory_is_forgotten_but_never_removed() {
    let mut fixture = Fixture::new();
    let archive = fixture.archive("owned");
    let package = fixture.install_into(&archive, "mods").unwrap();
    let file = fixture.game.join("mods/owned.plugin");
    let bytes = fs::read(&file).unwrap();
    fs::remove_dir_all(fixture.game.join("mods")).unwrap();
    fs::create_dir(fixture.game.join("mods")).unwrap();
    fs::write(&file, bytes).unwrap();

    fixture.uninstall(&package.id).unwrap();
    assert!(!file.exists());
    assert!(fixture.game.join("mods").is_dir());
    assert_eq!(fixture.count("created_directories"), 0);
}

#[test]
fn an_interrupted_install_removes_the_directories_it_created() {
    let mut fixture = Fixture::new();
    let archive = fixture.archive("partial");
    let error = fixture.interrupted_into(&archive, "mods/nested");
    assert!(
        error.to_string().contains("injected interruption"),
        "{error}"
    );
    assert!(!fixture.game.join("mods").exists());
    assert_eq!(fixture.count("pending_created_directories"), 0);
    assert_eq!(fixture.count("created_directories"), 0);
    assert_eq!(fixture.count("pending_transactions"), 0);
}

fn crashed_install(fixture: &Fixture, relative: &str, identity: Option<&str>) {
    let game = fixture.game.to_string_lossy().into_owned();
    let connection = &fixture.store.connection;
    connection
        .execute(
            "INSERT INTO pending_transactions(id,action,status,game_path,package_id)
             VALUES ('txn-crash','install','pending',?1,'txn-crash')",
            [&game],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO pending_created_directories(transaction_id,ord,relative_path,identity)
             VALUES ('txn-crash',0,?1,?2)",
            rusqlite::params![relative, identity],
        )
        .unwrap();
}

#[test]
fn an_unproven_creation_record_never_removes_a_directory() {
    let mut fixture = Fixture::new();
    crashed_install(&fixture, "mods", None);
    fs::create_dir(fixture.game.join("mods")).unwrap();
    fixture.recover();
    assert!(fixture.game.join("mods").is_dir());
    assert_eq!(fixture.count("pending_created_directories"), 0);
    assert_eq!(fixture.count("created_directories"), 0);
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn an_occupied_rollback_directory_moves_to_the_ledger() {
    let mut fixture = Fixture::new();
    let mods = fixture.game.join("mods");
    fs::create_dir(&mods).unwrap();
    let identity = super::directory::identity(&mods).unwrap();
    fs::write(mods.join("user.txt"), b"user").unwrap();
    crashed_install(&fixture, "mods", Some(&identity));

    fixture.recover();
    assert!(mods.join("user.txt").is_file());
    assert_eq!(fixture.count("created_directories"), 1);
    fs::remove_file(mods.join("user.txt")).unwrap();
    fixture.recover();
    assert!(!mods.exists());
    assert_eq!(fixture.count("created_directories"), 0);
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn a_recreated_directory_replaces_its_stale_record() {
    let mut fixture = Fixture::new();
    let first = fixture.archive("first");
    let second = fixture.archive("second");
    let mods = fixture.game.join("mods");
    let a = fixture.install_into(&first, "mods").unwrap();
    fs::write(mods.join("user.txt"), b"user").unwrap();
    fixture.uninstall(&a.id).unwrap();
    assert_eq!(fixture.count("created_directories"), 1);

    fs::remove_dir_all(&mods).unwrap();
    let b = fixture.install_into(&second, "mods").unwrap();
    fixture.uninstall(&b.id).unwrap();
    assert!(!mods.exists());
    assert_eq!(fixture.count("created_directories"), 0);
}

#[cfg(target_os = "linux")]
#[test]
fn directories_differing_only_in_case_are_separate_on_linux() {
    let mut fixture = Fixture::new();
    let upper = fixture.archive("upper");
    let lower = fixture.archive("lower");
    let a = fixture.install_into(&upper, "mods/Foo").unwrap();
    let b = fixture.install_into(&lower, "mods/foo").unwrap();
    assert_eq!(fixture.count("created_directories"), 3);
    fixture.uninstall(&a.id).unwrap();
    fixture.uninstall(&b.id).unwrap();
    assert!(!fixture.game.join("mods").exists());
}
