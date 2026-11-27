use super::{create, delete, list, set_enabled, set_priority, Placement, ProfilePackage};
use crate::error::{code_of, ErrorCode};
use crate::state::Store;
use crate::transactions::NativeInjectorPolicy;
use rusqlite::params;
use std::fs;
use tempfile::tempdir;

mod fixtures;

use fixtures::Fixture;

fn priorities(packages: &[ProfilePackage]) -> Vec<i64> {
    packages.iter().map(|package| package.priority).collect()
}

#[test]
fn add_game_creates_an_active_default_profile() {
    crate::test_support::seeded_roots(|roots| {
        let root = tempdir().expect("fixture");
        let game = root.path().join("game");
        fs::create_dir_all(game.join("bin/x64")).expect("bin");
        fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"game").expect("exe");
        let store = Store::open(&root.path().join("state/state.sqlite")).expect("state");
        store
            .add_game(roots, "cyberpunk-2077", &game, "test")
            .expect("register");
        let game = game.canonicalize().expect("canonical game");
        let profiles = list(&store, &game).expect("profiles");
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "default");
        assert!(profiles[0].active);
        assert!(profiles[0].packages.is_empty());
    });
}

#[test]
fn the_first_install_creates_the_default_profile() {
    let mut fixture = Fixture::new();
    let package = fixture.install("one", "one.plugin");
    let profiles = fixture.profiles();
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].name, "default");
    assert!(profiles[0].active);
    assert_eq!(
        profiles[0].packages,
        [ProfilePackage {
            package_id: package.id,
            priority: 0,
            enabled: true,
            shadowed_files: 0,
        }]
    );
}

#[test]
fn installs_append_memberships_at_next_priority() {
    let mut fixture = Fixture::new();
    let first = fixture.install("one", "one.plugin");
    let second = fixture.install("two", "two.plugin");
    let third = fixture.install("three", "three.plugin");
    let packages = fixture.profile("default").packages;
    assert_eq!(priorities(&packages), [2, 1, 0]);
    assert_eq!(
        packages
            .iter()
            .map(|package| package.package_id.as_str())
            .collect::<Vec<_>>(),
        [third.id.as_str(), second.id.as_str(), first.id.as_str()]
    );
}

#[test]
fn clone_copies_memberships_priorities_and_enabled_flags() {
    let mut fixture = Fixture::new();
    let first = fixture.install("one", "one.plugin");
    fixture.install("two", "two.plugin");
    set_enabled(
        &mut fixture.store,
        &fixture.game,
        "default",
        &first.id,
        false,
    )
    .expect("disable");
    let cloned =
        create(&mut fixture.store, &fixture.game, "cloned", Some("default")).expect("clone");
    assert_eq!(cloned.name, "cloned");
    assert!(!cloned.active);
    assert_eq!(cloned.packages, fixture.profile("default").packages);
    assert_eq!(
        priorities(&cloned.packages),
        priorities(&fixture.profile("default").packages)
    );
    assert!(!cloned.packages[1].enabled);
}

#[test]
fn create_refuses_an_existing_name_with_profile_exists() {
    let mut fixture = Fixture::new();
    create(&mut fixture.store, &fixture.game, "spare", None).expect("first create");
    let error =
        create(&mut fixture.store, &fixture.game, "spare", None).expect_err("duplicate name");
    assert_eq!(code_of(&error), Some(ErrorCode::ProfileExists));
    assert_eq!(fixture.counts("profiles"), 1);
}

#[test]
fn create_refuses_an_unknown_clone_source() {
    let mut fixture = Fixture::new();
    let error = create(&mut fixture.store, &fixture.game, "spare", Some("missing"))
        .expect_err("missing source");
    assert_eq!(code_of(&error), Some(ErrorCode::ProfileNotFound));
    assert_eq!(fixture.counts("profiles"), 0);
}

#[test]
fn profile_names_are_unique_per_game_and_reusable_for_another_game() {
    let mut fixture = Fixture::new();
    let other = fixture.other_game("other");
    create(&mut fixture.store, &fixture.game, "spare", None).expect("first game");
    let duplicate = create(&mut fixture.store, &fixture.game, "spare", None)
        .expect_err("duplicate within game");
    assert_eq!(code_of(&duplicate), Some(ErrorCode::ProfileExists));
    create(&mut fixture.store, &other, "spare", None).expect("other game");
    assert_eq!(fixture.profiles().len(), 1);
    assert_eq!(
        list(&fixture.store, &other).expect("other profiles").len(),
        1
    );
}

#[test]
fn a_second_profile_is_created_inactive_and_the_default_stays_active() {
    let mut fixture = Fixture::new();
    fixture.install("one", "one.plugin");
    let spare = create(&mut fixture.store, &fixture.game, "spare", None).expect("create");
    assert!(!spare.active);
    let profiles = fixture.profiles();
    assert!(profiles[0].active, "the active profile lists first");
    assert_eq!(profiles.iter().filter(|profile| profile.active).count(), 1);
    assert!(fixture.profile("default").active);
}

#[test]
fn delete_refuses_the_active_profile_without_changes() {
    let mut fixture = Fixture::new();
    fixture.install("one", "one.plugin");
    let error = delete(&mut fixture.store, &fixture.game, "default").expect_err("active");
    assert_eq!(code_of(&error), Some(ErrorCode::ProfileActive));
    assert_eq!(fixture.counts("profiles"), 1);
    assert_eq!(fixture.counts("profile_packages"), 1);
    assert_eq!(fixture.profile("default").packages.len(), 1);
}

#[test]
fn delete_refuses_the_only_remaining_profile_without_changes() {
    let mut fixture = Fixture::new();
    fixture.install("one", "one.plugin");
    fixture
        .store
        .connection
        .execute("UPDATE profiles SET active=0", [])
        .expect("deactivate fixture");
    let error = delete(&mut fixture.store, &fixture.game, "default").expect_err("last profile");
    assert!(error.to_string().contains("PROFILE_LAST"), "{error}");
    assert_eq!(fixture.counts("profiles"), 1);
    assert_eq!(fixture.counts("profile_packages"), 1);
}

#[test]
fn delete_keeps_a_shared_package_and_removes_an_exclusive_package() {
    let mut fixture = Fixture::new();
    let shared = fixture.install("shared", "shared.plugin");
    let spare = create(&mut fixture.store, &fixture.game, "spare", Some("default")).expect("clone");
    let game_text = fixture.game.to_string_lossy().to_string();
    fixture
        .store
        .connection
        .execute(
            "INSERT INTO managed_packages(id,game_path,archive_blake3)
             VALUES ('exclusive',?1,'hash')",
            params![game_text],
        )
        .expect("exclusive package");
    fixture
        .store
        .connection
        .execute(
            "INSERT INTO profile_packages(profile_id,package_id,priority,enabled)
             VALUES (?1,'exclusive',1,1)",
            [&spare.id],
        )
        .expect("exclusive membership");
    fixture
        .store
        .connection
        .execute(
            "INSERT INTO file_claims
             (package_id,game_path,root,path_key,relative_path,archive_entry,blake3,bytes)
             VALUES ('exclusive',?1,'game','x','x.archive','x.archive','aa',1)",
            params![game_text],
        )
        .expect("exclusive claim");
    for statement in [
        "INSERT INTO managed_package_releases(package_id,mod_id) VALUES ('exclusive','exclusive')",
        "INSERT INTO catalog_provenance(package_id,bucket,game_id,mod_id)
         VALUES ('exclusive','deps','test-game','exclusive')",
    ] {
        fixture
            .store
            .connection
            .execute(statement, [])
            .expect("exclusive metadata");
    }
    delete(&mut fixture.store, &fixture.game, "spare").expect("delete spare");
    assert_eq!(fixture.counts("profiles"), 1);
    assert!(fixture.profile("default").active);
    assert_eq!(fixture.profile("default").packages.len(), 1);
    assert_eq!(fixture.profile("default").packages[0].package_id, shared.id);
    for (table, column) in [
        ("managed_packages", "id"),
        ("profile_packages", "package_id"),
        ("file_claims", "package_id"),
        ("managed_package_releases", "package_id"),
        ("catalog_provenance", "package_id"),
    ] {
        let remaining: i64 = fixture
            .store
            .connection
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE {column}='exclusive'"),
                [],
                |row| row.get(0),
            )
            .expect("remaining row");
        assert_eq!(remaining, 0, "{table} kept an exclusive package row");
    }
}

#[test]
fn rank_moves_a_package_and_renumbers_priorities_contiguously() {
    let mut fixture = Fixture::new();
    let first = fixture.install("one", "one.plugin");
    let second = fixture.install("two", "two.plugin");
    let third = fixture.install("three", "three.plugin");
    set_priority(
        &mut fixture.store,
        &fixture.game,
        "default",
        &first.id,
        Placement::Rank(1),
    )
    .expect("rank one");
    let packages = fixture.profile("default").packages;
    assert_eq!(
        packages
            .iter()
            .map(|package| package.package_id.as_str())
            .collect::<Vec<_>>(),
        [first.id.as_str(), third.id.as_str(), second.id.as_str()]
    );
    assert_eq!(priorities(&packages), [2, 1, 0]);
    let mut ordered: Vec<i64> = priorities(&packages);
    ordered.sort_unstable();
    assert_eq!(ordered, [0, 1, 2]);
}

#[test]
fn before_places_a_package_immediately_above_another() {
    let mut fixture = Fixture::new();
    let first = fixture.install("one", "one.plugin");
    let second = fixture.install("two", "two.plugin");
    let third = fixture.install("three", "three.plugin");
    set_priority(
        &mut fixture.store,
        &fixture.game,
        "default",
        &first.id,
        Placement::Before(third.id.clone()),
    )
    .expect("before");
    let packages = fixture.profile("default").packages;
    assert_eq!(
        packages
            .iter()
            .map(|package| package.package_id.as_str())
            .collect::<Vec<_>>(),
        [first.id.as_str(), third.id.as_str(), second.id.as_str()]
    );
    assert_eq!(priorities(&packages), [2, 1, 0]);
}

#[test]
fn after_places_a_package_immediately_below_another() {
    let mut fixture = Fixture::new();
    let first = fixture.install("one", "one.plugin");
    let second = fixture.install("two", "two.plugin");
    let third = fixture.install("three", "three.plugin");
    set_priority(
        &mut fixture.store,
        &fixture.game,
        "default",
        &first.id,
        Placement::After(second.id.clone()),
    )
    .expect("after");
    let packages = fixture.profile("default").packages;
    assert_eq!(
        packages
            .iter()
            .map(|package| package.package_id.as_str())
            .collect::<Vec<_>>(),
        [third.id.as_str(), second.id.as_str(), first.id.as_str()]
    );
    assert_eq!(priorities(&packages), [2, 1, 0]);
}

#[test]
fn invalid_ranks_are_refused_without_changes() {
    let mut fixture = Fixture::new();
    let first = fixture.install("one", "one.plugin");
    fixture.install("two", "two.plugin");
    let before = fixture.profile("default").packages;
    for rank in [0, 3] {
        let error = set_priority(
            &mut fixture.store,
            &fixture.game,
            "default",
            &first.id,
            Placement::Rank(rank),
        )
        .expect_err("invalid rank");
        assert_eq!(code_of(&error), Some(ErrorCode::InvalidArguments));
        assert_eq!(fixture.profile("default").packages, before);
    }
}

#[test]
fn disabling_and_enabling_flip_only_the_membership_flag() {
    let mut fixture = Fixture::new();
    let first = fixture.install("one", "one.plugin");
    fixture.install("two", "two.plugin");
    let before = fixture.profile("default").packages;
    set_enabled(
        &mut fixture.store,
        &fixture.game,
        "default",
        &first.id,
        false,
    )
    .expect("disable");
    let disabled = fixture.profile("default").packages;
    assert!(!disabled[1].enabled);
    assert_eq!(priorities(&disabled), priorities(&before));
    assert_eq!(disabled[0], before[0]);
    assert_eq!(disabled[1].package_id, first.id);
    set_enabled(
        &mut fixture.store,
        &fixture.game,
        "default",
        &first.id,
        true,
    )
    .expect("enable");
    assert_eq!(fixture.profile("default").packages, before);
}

#[test]
fn uninstall_removes_the_package_membership() {
    let mut fixture = Fixture::new();
    let package = fixture.install("one", "one.plugin");
    let other = fixture.install("two", "two.plugin");
    fixture
        .store
        .uninstall(&fixture.game, &package.id)
        .expect("uninstall");
    let packages = fixture.profile("default").packages;
    assert_eq!(
        packages
            .iter()
            .map(|package| package.package_id.as_str())
            .collect::<Vec<_>>(),
        [other.id.as_str()]
    );
    assert_eq!(priorities(&packages), [1]);
    assert_eq!(fixture.counts("profile_packages"), 1);
}

#[test]
fn update_keeps_the_package_membership_and_priority() {
    let mut fixture = Fixture::new();
    let (base, release) = fixture.install_catalog("base-1", "base");
    let (top, _) = fixture.install_catalog("top", "top");
    set_priority(
        &mut fixture.store,
        &fixture.game,
        "default",
        &base.id,
        Placement::Rank(1),
    )
    .expect("raise base");
    let archive = fixture.archive("base-2", "base.plugin", b"base-2");
    let plan = fixture.plan(&archive);
    let updated = fixture
        .store
        .update_catalog_release(
            &fixture.game,
            &base.id,
            &archive,
            &plan,
            &release,
            NativeInjectorPolicy::Refuse,
            &[],
        )
        .expect("update");
    assert_ne!(updated.id, base.id);
    let packages = fixture.profile("default").packages;
    assert_eq!(packages.len(), 2);
    assert_eq!(packages[0].package_id, updated.id);
    assert_eq!(packages[0].priority, 1);
    assert!(packages[0].enabled);
    assert_eq!(packages[1].package_id, top.id);
    assert_eq!(packages[1].priority, 0);
    assert_eq!(priorities(&packages), [1, 0]);
}

#[test]
fn managed_files_record_the_active_profile_id() {
    let mut fixture = Fixture::new();
    fixture.install("one", "one.plugin");
    let active = fixture.profile("default").id;
    let mut statement = fixture
        .store
        .connection
        .prepare("SELECT profile_id FROM managed_files")
        .expect("managed files");
    let recorded = statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("rows");
    assert_eq!(recorded, [active]);
}
