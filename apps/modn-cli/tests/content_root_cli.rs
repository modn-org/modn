//! End-to-end coverage for API 2 modules that declare a content root.
//!
//! Every command runs the real `modn` binary against a temporary game folder,
//! a temporary appdata base and a temporary state directory. `MODN_APPDATA`
//! and `APPDATA` both point at the temporary base, so no test reads or writes
//! the user's real `%APPDATA%`.

#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "support/content_root.rs"]
mod support;
#[allow(
    dead_code,
    reason = "shared fixture exposes helpers for different test crates"
)]
#[path = "support/tree.rs"]
mod tree;

use serde_json::json;
use std::fs;
use std::path::Path;
use support::{
    assert_content_ownership, error, expected_content_root, owned_files, Fixture, GAME_ID,
};

#[test]
fn content_root_install_dry_run_list_and_uninstall_stay_outside_the_game() {
    let fixture = Fixture::new();
    fixture.register();
    let game_before = tree::snapshot(&fixture.game);
    let (archive, _) = fixture.archive(
        "mod-one",
        &[("core.lua", b"version one"), ("Extra/readme.txt", b"extra")],
    );

    let dry_run = fixture.json(&[
        "--json",
        "install",
        archive.to_str().expect("archive"),
        "--dry-run",
        "--path",
        fixture.game.to_str().expect("game"),
    ]);
    assert_eq!(dry_run["status"], "reviewable", "{dry_run}");
    assert_eq!(dry_run["operations"][0]["root"], "content");
    assert_eq!(dry_run["operations"][1]["root"], "content");
    assert!(dry_run["blockers"].as_array().is_some_and(Vec::is_empty));
    assert_eq!(
        Path::new(dry_run["content_root"].as_str().expect("content root")),
        expected_content_root(&fixture)
    );
    assert!(
        !fixture.appdata.join("Fixture").exists(),
        "dry run must not create the content root"
    );
    assert_eq!(tree::snapshot(&fixture.game), game_before);
    assert_eq!(owned_files(&fixture.database()), 0);

    let installed = fixture.json(&[
        "--json",
        "install",
        archive.to_str().expect("archive"),
        "--yes",
        "--path",
        fixture.game.to_str().expect("game"),
    ]);
    let id = installed["id"].as_str().expect("package id").to_owned();
    assert_eq!(tree::snapshot(&fixture.game), game_before);
    assert_eq!(
        fs::read(fixture.content("Pkg/core.lua")).expect("content file"),
        b"version one"
    );
    assert_eq!(
        fs::read(fixture.content("Pkg/Extra/readme.txt")).expect("nested content file"),
        b"extra"
    );
    assert!(
        !fixture.game.join("Pkg").exists(),
        "game root must stay empty"
    );
    for file in installed["files"].as_array().expect("files") {
        assert_eq!(file["root"], "content");
    }
    assert_content_ownership(&fixture.database(), &fixture.content_root());

    let listed = fixture.json(&[
        "--json",
        "list",
        "--path",
        fixture.game.to_str().expect("game"),
    ]);
    assert_eq!(listed.as_array().map(Vec::len), Some(1));
    assert_eq!(listed[0]["id"], id);
    for file in listed[0]["files"].as_array().expect("listed files") {
        assert_eq!(file["root"], "content");
        assert!(!fixture
            .game
            .join(file["destination"].as_str().unwrap())
            .exists());
    }

    fixture.ok(&[
        "uninstall",
        &id,
        "--yes",
        "--path",
        fixture.game.to_str().expect("game"),
    ]);
    assert!(!fixture.content("Pkg/core.lua").exists());
    assert!(!fixture.content("Pkg/Extra/readme.txt").exists());
    assert!(
        fixture
            .content_root()
            .read_dir()
            .map_or(true, |mut entries| entries.next().is_none()),
        "the content root holds no Modn files after uninstall"
    );
    assert_eq!(tree::snapshot(&fixture.game), game_before);
    assert_eq!(owned_files(&fixture.database()), 0);
}

#[test]
fn adoption_requires_acknowledgement_and_force_removes_only_owned_content_files() {
    let fixture = Fixture::new();
    fixture.register();
    let game_before = tree::snapshot(&fixture.game);
    let (archive, _) = fixture.archive("adopt-one", &[("core.lua", b"manual")]);
    fs::create_dir_all(fixture.content("Pkg")).expect("content folder");
    fs::write(fixture.content("Pkg/core.lua"), b"manual").expect("manual file");
    fs::write(fixture.content("notes.txt"), b"unmanaged").expect("unmanaged file");

    let preview = fixture.json(&[
        "--json",
        "adopt",
        archive.to_str().expect("archive"),
        "--dry-run",
        "--path",
        fixture.game.to_str().expect("game"),
    ]);
    assert_eq!(preview["status"], "verification_required", "{preview}");
    assert_eq!(preview["plan"]["operations"][0]["root"], "content");
    assert_eq!(
        Path::new(
            preview["plan"]["content_root"]
                .as_str()
                .expect("content root")
        ),
        expected_content_root(&fixture)
    );

    let unacknowledged = fixture.run(&[
        "adopt",
        archive.to_str().expect("archive"),
        "--yes",
        "--path",
        fixture.game.to_str().expect("game"),
    ]);
    assert!(!unacknowledged.status.success());
    assert!(
        error(&unacknowledged).contains("both --acknowledge and --yes"),
        "{}",
        error(&unacknowledged)
    );
    assert_eq!(owned_files(&fixture.database()), 0);

    let adopted = fixture.json(&[
        "--json",
        "adopt",
        archive.to_str().expect("archive"),
        "--acknowledge",
        "--yes",
        "--path",
        fixture.game.to_str().expect("game"),
    ]);
    let id = adopted["id"].as_str().expect("package id").to_owned();
    assert_eq!(
        fs::read(fixture.content("Pkg/core.lua")).expect("file"),
        b"manual"
    );
    assert_eq!(
        fs::read(fixture.content("notes.txt")).expect("unmanaged"),
        b"unmanaged"
    );
    assert_eq!(tree::snapshot(&fixture.game), game_before);
    assert_content_ownership(&fixture.database(), &fixture.content_root());

    fs::write(fixture.content("Pkg/core.lua"), b"edited by the user").expect("modified file");
    let strict = fixture.run(&[
        "uninstall",
        &id,
        "--yes",
        "--path",
        fixture.game.to_str().expect("game"),
    ]);
    assert!(!strict.status.success(), "a changed owned file must refuse");
    assert!(fixture.content("Pkg/core.lua").exists());

    fixture.ok(&[
        "uninstall",
        &id,
        "--force",
        "--yes",
        "--path",
        fixture.game.to_str().expect("game"),
    ]);
    assert!(
        !fixture.content("Pkg/core.lua").exists(),
        "--force removes only recorded Modn-owned files"
    );
    assert_eq!(
        fs::read(fixture.content("notes.txt")).expect("unmanaged"),
        b"unmanaged"
    );
    assert_eq!(tree::snapshot(&fixture.game), game_before);
    assert_eq!(owned_files(&fixture.database()), 0);
}

#[test]
fn catalog_update_under_content_root_replaces_files_and_rolls_back_a_fault() {
    let mut fixture = Fixture::new();
    fixture.register();
    fixture.bucket();
    let game_before = tree::snapshot(&fixture.game);
    let (v1, v1_hash) = fixture.archive(
        "fixture-1.0",
        &[("core.lua", b"v1 core"), ("retired.lua", b"v1 only")],
    );
    let (v2, v2_hash) = fixture.archive(
        "fixture-2.0",
        &[("core.lua", b"v2 core"), ("added.lua", b"v2 only")],
    );
    fixture.release("fixture.json", "1.0", &v1_hash);

    let installed = fixture.json(&[
        "--json",
        "install",
        "fixture",
        "--file",
        v1.to_str().expect("v1 archive"),
        "--yes",
        "--path",
        fixture.game.to_str().expect("game"),
    ]);
    assert_eq!(installed["package"], "fixture-bucket/content-game/fixture");
    assert_eq!(
        fs::read(fixture.content("Pkg/core.lua")).expect("v1"),
        b"v1 core"
    );
    assert_eq!(
        fs::read(fixture.content("Pkg/retired.lua")).expect("v1 only"),
        b"v1 only"
    );
    assert_eq!(tree::snapshot(&fixture.game), game_before);
    assert_content_ownership(&fixture.database(), &fixture.content_root());

    fixture.release("fixture@1.0.json", "1.0", &v1_hash);
    fixture.release("fixture.json", "2.0", &v2_hash);
    let preview = fixture.json(&[
        "--json",
        "update",
        "fixture",
        "--file",
        v2.to_str().expect("v2 archive"),
        "--dry-run",
        "--path",
        fixture.game.to_str().expect("game"),
    ]);
    assert_eq!(preview["installed"]["version"], "1.0", "{preview}");
    assert_eq!(preview["target"]["version"], "2.0", "{preview}");
    assert_eq!(preview["files"]["added"], json!(["Pkg/added.lua"]));
    assert_eq!(preview["files"]["replaced"], json!(["Pkg/core.lua"]));
    assert_eq!(preview["files"]["removed"], json!(["Pkg/retired.lua"]));
    assert_eq!(preview["plan"]["operations"][0]["root"], "content");
    assert_eq!(
        Path::new(
            preview["plan"]["content_root"]
                .as_str()
                .expect("content root")
        ),
        expected_content_root(&fixture)
    );
    assert_eq!(
        fs::read(fixture.content("Pkg/core.lua")).expect("v1 intact"),
        b"v1 core"
    );
    assert!(!fixture.content("Pkg/added.lua").exists());

    let failed = fixture.run_env(
        &[
            "update",
            "fixture",
            "--file",
            v2.to_str().expect("v2 archive"),
            "--yes",
            "--path",
            fixture.game.to_str().expect("game"),
        ],
        &[("MODN_TEST_UPDATE_FAULT_AFTER", "1")],
    );
    assert!(!failed.status.success(), "the injected fault must fail");
    assert!(
        error(&failed).contains("injected interruption"),
        "{}",
        error(&failed)
    );
    assert_eq!(
        fs::read(fixture.content("Pkg/core.lua")).expect("rollback keeps v1"),
        b"v1 core"
    );
    assert_eq!(
        fs::read(fixture.content("Pkg/retired.lua")).expect("rollback keeps v1 only"),
        b"v1 only"
    );
    assert!(!fixture.content("Pkg/added.lua").exists());
    assert_eq!(tree::snapshot(&fixture.game), game_before);
    let database = fixture.database();
    let pending: i64 = database
        .query_row(
            "SELECT COUNT(*) FROM pending_transactions WHERE status <> 'committed'",
            [],
            |row| row.get(0),
        )
        .expect("pending count");
    assert_eq!(pending, 0, "automatic rollback must retire the journal");
    let rollback_rows: i64 = database
        .query_row("SELECT COUNT(*) FROM managed_files", [], |row| row.get(0))
        .expect("rows");
    assert_eq!(rollback_rows, 2, "v1 ownership is restored");
    assert_content_ownership(&database, &fixture.content_root());

    let listed = fixture.json(&[
        "--json",
        "list",
        "--path",
        fixture.game.to_str().expect("game"),
    ]);
    assert_eq!(
        listed[0]["catalog"],
        json!({"bucket": "fixture-bucket", "game_id": GAME_ID, "mod_id": "fixture"})
    );
    assert_eq!(listed[0]["files"].as_array().map(Vec::len), Some(2));

    fixture.ok(&[
        "update",
        "fixture",
        "--file",
        v2.to_str().expect("v2 archive"),
        "--yes",
        "--path",
        fixture.game.to_str().expect("game"),
    ]);
    assert_eq!(
        fs::read(fixture.content("Pkg/core.lua")).expect("v2"),
        b"v2 core"
    );
    assert_eq!(
        fs::read(fixture.content("Pkg/added.lua")).expect("v2 only"),
        b"v2 only"
    );
    assert!(!fixture.content("Pkg/retired.lua").exists());
    assert_eq!(tree::snapshot(&fixture.game), game_before);
    assert_content_ownership(&fixture.database(), &fixture.content_root());
    assert_eq!(owned_files(&fixture.database()), 2);
}
