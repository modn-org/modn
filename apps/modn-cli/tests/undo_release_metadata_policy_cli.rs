//! Undo enforces stored approvals and the native-injector policy, and refuses
//! records that lack the release declarations a restore needs.

#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "support/catalog.rs"]
mod support;

#[path = "common/release_metadata.rs"]
mod release_metadata;

use crate::support::{error, Fixture};
use release_metadata::{
    archive, cache, error_code, history, https, id_of, install, json_of, remove_declarations, run,
    state_database, write_manifest,
};
use serde_json::json;
use std::fs;

#[test]
fn undo_of_an_update_and_an_uninstall_enforces_the_native_injector_policy() {
    let fixture = Fixture::new();
    let (v1, v1_hash) = archive(&fixture, "native-v1", &[("winmm.dll", "one")]);
    let (v2, v2_hash) = archive(&fixture, "native-v2", &[("winmm.dll", "two")]);
    let v1_url = "https://example.invalid/native-1.0.zip";
    let v2_url = "https://example.invalid/native-2.0.zip";
    write_manifest(
        &fixture,
        "native",
        "native-injector-game",
        "injector",
        "1.0",
        &v1_hash,
        &[],
        &[],
        true,
        https(v1_url),
    );
    fixture.register_native();
    cache(
        &fixture,
        "native-injector-game",
        "injector",
        "1.0",
        v1_url,
        &v1,
    );
    let installed = install(&fixture, "injector", &v1, &["--allow-native-injector"]);
    assert!(installed.status.success(), "{}", error(&installed));
    let library = fixture.native_game.join("winmm.dll");
    assert_eq!(fs::read_to_string(&library).expect("v1"), "one");

    write_manifest(
        &fixture,
        "native",
        "native-injector-game",
        "injector",
        "2.0",
        &v2_hash,
        &[],
        &[],
        true,
        https(v2_url),
    );
    let updated = run(&fixture, &["--json", "update", "injector", "--yes"], &[&v2]);
    assert!(updated.status.success(), "{}", error(&updated));
    let update = id_of(&history(&fixture)[0]);

    state_database(&fixture)
        .execute("DELETE FROM mod_approvals", [])
        .expect("drop stored approval");
    let refused = fixture.run(&["--json", "undo", &update, "--yes"]);
    assert!(!refused.status.success(), "{}", error(&refused));
    assert_eq!(error_code(&refused), "NATIVE_INJECTOR_NOT_ALLOWED");
    assert_eq!(fs::read_to_string(&library).expect("v2 stays"), "two");
    let allowed = fixture.run(&[
        "--json",
        "undo",
        &update,
        "--yes",
        "--allow-native-injector",
    ]);
    assert!(allowed.status.success(), "{}", error(&allowed));
    assert_eq!(fs::read_to_string(&library).expect("v1 restored"), "one");

    assert!(fixture
        .run(&["uninstall", "injector", "--yes"])
        .status
        .success());
    let uninstall = id_of(&history(&fixture)[0]);
    let refused = fixture.run(&["--json", "undo", &uninstall, "--yes"]);
    assert!(!refused.status.success(), "{}", error(&refused));
    assert_eq!(error_code(&refused), "NATIVE_INJECTOR_NOT_ALLOWED");
    assert!(!library.exists());
    let allowed = fixture.run(&[
        "--json",
        "undo",
        &uninstall,
        "--yes",
        "--allow-native-injector",
    ]);
    assert!(allowed.status.success(), "{}", error(&allowed));
    assert_eq!(fs::read_to_string(&library).expect("restored"), "one");
}

#[test]
fn undo_refuses_a_record_without_release_declarations() {
    let fixture = Fixture::new();
    let (v1, v1_hash) = archive(
        &fixture,
        "v1",
        &[("Demo/package/plugins/example.plugin", "one")],
    );
    let url = "https://example.invalid/example-1.0.zip";
    write_manifest(
        &fixture,
        "deps",
        "test-game",
        "example",
        "1.0",
        &v1_hash,
        &[],
        &[],
        false,
        https(url),
    );
    fixture.register();
    cache(&fixture, "test-game", "example", "1.0", url, &v1);
    assert!(install(&fixture, "example", &v1, &[]).status.success());
    assert!(fixture
        .run(&["uninstall", "example", "--yes"])
        .status
        .success());
    let target = id_of(&history(&fixture)[0]);
    remove_declarations(&fixture, &target, "before");

    let before = history(&fixture);
    let refused = fixture.run(&["--json", "undo", &target, "--yes"]);
    assert!(!refused.status.success(), "{}", error(&refused));
    assert_eq!(error_code(&refused), "UNDO_FAILED");
    let message = error(&refused);
    assert!(message.contains("install the release again"), "{message}");
    assert!(!fixture.game.join("addons/example.plugin").exists());
    assert_eq!(history(&fixture), before);
}

#[test]
fn undo_of_an_update_refuses_a_record_without_release_declarations() {
    let fixture = Fixture::new();
    let (v1, v1_hash) = archive(
        &fixture,
        "v1",
        &[("Demo/package/plugins/example.plugin", "one")],
    );
    let (v2, v2_hash) = archive(
        &fixture,
        "v2",
        &[("Demo/package/plugins/example.plugin", "two")],
    );
    let url = "https://example.invalid/example-1.0.zip";
    write_manifest(
        &fixture,
        "deps",
        "test-game",
        "example",
        "1.0",
        &v1_hash,
        &[],
        &[],
        false,
        https(url),
    );
    fixture.register();
    assert!(install(&fixture, "example", &v1, &[]).status.success());
    cache(&fixture, "test-game", "example", "1.0", url, &v1);
    write_manifest(
        &fixture,
        "deps",
        "test-game",
        "example",
        "2.0",
        &v2_hash,
        &[],
        &[],
        false,
        json!({"kind": "manual", "filename": "v2.zip"}),
    );
    let updated = run(&fixture, &["--json", "update", "example", "--yes"], &[&v2]);
    assert!(updated.status.success(), "{}", error(&updated));
    let target = id_of(&history(&fixture)[0]);
    remove_declarations(&fixture, &target, "before");

    let addon = fixture.game.join("addons/example.plugin");
    let before = history(&fixture);
    let refused = fixture.run(&["--json", "undo", &target, "--yes"]);
    assert!(!refused.status.success(), "{}", error(&refused));
    assert_eq!(error_code(&refused), "UNDO_FAILED");
    assert!(error(&refused).contains("install the release again"));
    assert_eq!(fs::read_to_string(&addon).expect("v2 stays"), "two");
    assert_eq!(history(&fixture), before);
}

#[test]
fn history_reads_an_older_state_without_release_declarations() {
    let fixture = Fixture::new();
    let (package, digest) = fixture.archive();
    fixture.bucket("deps", "test-game", "example", &digest, &[]);
    fixture.register();
    assert!(install(&fixture, "example", &package, &[]).status.success());
    let database = state_database(&fixture);
    database
        .execute_batch(
            "ALTER TABLE managed_package_releases DROP COLUMN catalog;
             ALTER TABLE managed_package_releases DROP COLUMN declarations;",
        )
        .expect("remove additive columns");
    drop(database);

    let database = fixture.state.join("state.sqlite");
    let before = fs::read(&database).expect("state bytes");
    let operations = json_of(&fixture.run(&["--json", "history"]));
    assert_eq!(
        operations.as_array().expect("history").len(),
        1,
        "history reads the older state"
    );
    assert_eq!(
        fs::read(&database).expect("state bytes"),
        before,
        "read-only history writes nothing"
    );
    let connection = rusqlite::Connection::open(&database).expect("state");
    let columns: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('managed_package_releases')
             WHERE name IN ('catalog', 'declarations')",
            [],
            |row| row.get(0),
        )
        .expect("column count");
    assert_eq!(columns, 0, "read-only history adds no columns");
}
