//! A stored approval is keyed by game path, bucket and mod ID, so approving a
//! mod of one bucket never approves a same-named mod of another bucket.

#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "support/catalog.rs"]
mod support;

#[path = "common/release_metadata.rs"]
mod release_metadata;

use crate::support::{error, Fixture};
use release_metadata::{
    archive, cache, error_code, history, https, id_of, install, state_database, tool_module,
    write_manifest,
};
use serde_json::{json, Value};
use std::process::Output;

const NATIVE_GAME: &str = "native-injector-game";
const TOOL_GAME: &str = "test-game";
const SHARED: &str = "shared";
const COMPILER: &str = "engine/tools/scc.exe";

fn package_id(output: &Output) -> String {
    let value: Value = serde_json::from_slice(&output.stdout).expect("install JSON");
    value["installed"]["id"]
        .as_str()
        .expect("package ID")
        .to_owned()
}

fn approval_rows(fixture: &Fixture, mod_id: &str) -> Vec<(String, i64)> {
    let database = state_database(fixture);
    let mut statement = database
        .prepare(
            "SELECT bucket, native_injector FROM mod_approvals
             WHERE mod_id=?1 ORDER BY bucket",
        )
        .expect("approval query");
    statement
        .query_map([mod_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("approval rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("read approvals")
}

#[test]
fn native_approval_of_one_bucket_does_not_cover_another_bucket() {
    let fixture = Fixture::new();
    fixture.register_native();
    let (archive, digest) = fixture.native_archive();
    for bucket in ["first", "second"] {
        write_manifest(
            &fixture,
            bucket,
            NATIVE_GAME,
            SHARED,
            "1.0",
            &digest,
            &[],
            &[],
            true,
            json!({"kind": "manual", "filename": "shared.zip"}),
        );
    }

    let installed = install(
        &fixture,
        &format!("first/{NATIVE_GAME}/{SHARED}"),
        &archive,
        &["--allow-native-injector"],
    );
    assert!(installed.status.success(), "{}", error(&installed));
    let removed = fixture.run(&["uninstall", &package_id(&installed), "--yes"]);
    assert!(removed.status.success(), "{}", error(&removed));

    let refused = install(
        &fixture,
        &format!("second/{NATIVE_GAME}/{SHARED}"),
        &archive,
        &[],
    );
    assert!(
        !refused.status.success(),
        "the first bucket's approval must not cover the second bucket"
    );
    assert_eq!(error_code(&refused), "NATIVE_INJECTOR_NOT_ALLOWED");
    assert!(!fixture.native_game.join("winmm.dll").exists());

    let approved = install(
        &fixture,
        &format!("second/{NATIVE_GAME}/{SHARED}"),
        &archive,
        &["--allow-native-injector"],
    );
    assert!(approved.status.success(), "{}", error(&approved));
    assert!(fixture.native_game.join("winmm.dll").is_file());
    assert_eq!(
        approval_rows(&fixture, SHARED),
        [("first".to_owned(), 1), ("second".to_owned(), 1)]
    );
}

#[test]
fn executable_approval_of_one_bucket_does_not_cover_another_bucket() {
    let fixture = Fixture::new();
    tool_module(&fixture);
    fixture.register();
    let (archive, digest) = archive(&fixture, "tool", &[(COMPILER, "compiler")]);
    for bucket in ["first", "second"] {
        write_manifest(
            &fixture,
            bucket,
            TOOL_GAME,
            SHARED,
            "1.0",
            &digest,
            &[],
            &[COMPILER],
            false,
            json!({"kind": "manual", "filename": "shared.zip"}),
        );
    }

    let installed = install(
        &fixture,
        &format!("first/{TOOL_GAME}/{SHARED}"),
        &archive,
        &["--allow-executable", COMPILER],
    );
    assert!(installed.status.success(), "{}", error(&installed));
    let removed = fixture.run(&["uninstall", &package_id(&installed), "--yes"]);
    assert!(removed.status.success(), "{}", error(&removed));
    assert!(!fixture.game.join(COMPILER).exists());

    let refused = install(
        &fixture,
        &format!("second/{TOOL_GAME}/{SHARED}"),
        &archive,
        &[],
    );
    assert!(
        !refused.status.success(),
        "the first bucket's approval must not cover the second bucket"
    );
    assert_eq!(error_code(&refused), "APPROVAL_REQUIRED");
    assert!(!fixture.game.join(COMPILER).exists());

    let approved = install(
        &fixture,
        &format!("second/{TOOL_GAME}/{SHARED}"),
        &archive,
        &["--allow-executable", COMPILER],
    );
    assert!(approved.status.success(), "{}", error(&approved));
    assert!(fixture.game.join(COMPILER).is_file());
    assert_eq!(
        approval_rows(&fixture, SHARED),
        [("first".to_owned(), 0), ("second".to_owned(), 0)]
    );
}

/// Install and uninstall one bucket's native mod from its cached https release
/// and return the ID of the recorded uninstall operation.
fn uninstall_native(fixture: &Fixture, bucket: &str, archive: &std::path::Path) -> String {
    let installed = install(
        fixture,
        &format!("{bucket}/{NATIVE_GAME}/{SHARED}"),
        archive,
        &["--allow-native-injector"],
    );
    assert!(installed.status.success(), "{}", error(&installed));
    let removed = fixture.run(&["uninstall", &package_id(&installed), "--yes"]);
    assert!(removed.status.success(), "{}", error(&removed));
    id_of(&history(fixture)[0])
}

fn two_native_buckets() -> (Fixture, std::path::PathBuf, [String; 2]) {
    let fixture = Fixture::new();
    fixture.register_native();
    let (archive, digest) = fixture.native_archive();
    let urls = [
        ("first", "https://example.invalid/first-1.0.zip"),
        ("second", "https://example.invalid/second-1.0.zip"),
    ];
    for (bucket, url) in urls {
        write_manifest(
            &fixture,
            bucket,
            NATIVE_GAME,
            SHARED,
            "1.0",
            &digest,
            &[],
            &[],
            true,
            https(url),
        );
        cache(&fixture, NATIVE_GAME, SHARED, "1.0", url, &archive);
    }
    let undo = urls.map(|(bucket, _)| uninstall_native(&fixture, bucket, &archive));
    (fixture, archive, undo)
}

fn forget_approval(fixture: &Fixture, bucket: &str) {
    state_database(fixture)
        .execute(
            "DELETE FROM mod_approvals WHERE bucket=?1 AND mod_id=?2",
            [bucket, SHARED],
        )
        .expect("remove one bucket's approval");
}

#[test]
fn undo_uninstall_refuses_another_buckets_approval() {
    let (fixture, _archive, undo) = two_native_buckets();
    forget_approval(&fixture, "second");

    let refused = fixture.run(&["--json", "undo", &undo[1], "--yes"]);
    assert!(
        !refused.status.success(),
        "the first bucket's approval must not restore the second bucket's mod"
    );
    assert_eq!(error_code(&refused), "NATIVE_INJECTOR_NOT_ALLOWED");
    assert!(!fixture.native_game.join("winmm.dll").exists());
}

#[test]
fn undo_uninstall_uses_the_approval_of_its_own_bucket() {
    let (fixture, _archive, undo) = two_native_buckets();
    assert_eq!(
        approval_rows(&fixture, SHARED),
        [("first".to_owned(), 1), ("second".to_owned(), 1)]
    );
    forget_approval(&fixture, "first");

    let restored = fixture.run(&["--json", "undo", &undo[1], "--yes"]);
    assert!(restored.status.success(), "{}", error(&restored));
    assert!(fixture.native_game.join("winmm.dll").is_file());
}
