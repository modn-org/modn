#[path = "support/catalog.rs"]
mod support;

#[path = "support/git_fixture.rs"]
mod git_fixture;

#[path = "support/git_source_helpers.rs"]
mod helpers;

#[path = "support/main_bucket.rs"]
mod main_bucket;

use git_fixture::repository;
use helpers::{add_bucket, cache_archives, installed_plugin, run, write_bucket};
use serde_json::{json, Value};
use std::fs;
use support::{error, Fixture};

#[test]
fn git_source_update_moves_between_commits_and_the_same_commit_is_current() {
    let fixture = Fixture::new();
    let repository = repository(fixture.temp.path());
    add_bucket(&fixture, fixture.temp.path(), &repository.commit);
    fixture.register();
    let game = fixture.game.to_str().expect("game path");
    let installed = run(
        &fixture,
        &repository.base,
        &[
            "--json",
            "install",
            "test-game/git-package",
            "--path",
            game,
            "--yes",
        ],
    );
    assert!(installed.status.success(), "{}", error(&installed));
    assert_eq!(
        fs::read(installed_plugin(&fixture)).expect("installed plugin"),
        b"pinned-release"
    );

    write_bucket(fixture.temp.path(), &repository.later);
    let planned = run(
        &fixture,
        &repository.base,
        &[
            "--json",
            "update",
            "git-package",
            "--path",
            game,
            "--dry-run",
            "--download",
        ],
    );
    assert!(planned.status.success(), "{}", error(&planned));
    let report: Value = serde_json::from_slice(&planned.stdout).expect("dry-run JSON");
    assert_eq!(report["up_to_date"], false);
    assert_eq!(
        report["files"]["replaced"],
        json!(["addons/git-source.plugin"])
    );
    assert_eq!(
        fs::read(installed_plugin(&fixture)).expect("unchanged plugin"),
        b"pinned-release"
    );

    let updated = run(
        &fixture,
        &repository.base,
        &["--json", "update", "git-package", "--path", game, "--yes"],
    );
    assert!(updated.status.success(), "{}", error(&updated));
    assert_eq!(
        fs::read(installed_plugin(&fixture)).expect("updated plugin"),
        b"later-release"
    );

    let current = run(
        &fixture,
        &repository.base,
        &[
            "--json",
            "update",
            "git-package",
            "--path",
            game,
            "--dry-run",
        ],
    );
    assert!(current.status.success(), "{}", error(&current));
    let report: Value = serde_json::from_slice(&current.stdout).expect("current JSON");
    assert_eq!(report["up_to_date"], true);
    assert_eq!(report["archive_needed"], Value::Null);
}

#[test]
fn git_source_update_accepts_a_supplied_zip() {
    let fixture = Fixture::new();
    let repository = repository(fixture.temp.path());
    add_bucket(&fixture, fixture.temp.path(), &repository.commit);
    fixture.register();
    let game = fixture.game.to_str().expect("game path");
    let install = ["install", "test-game/git-package", "--path", game, "--yes"];
    let installed = run(&fixture, &repository.base, &install);
    assert!(installed.status.success(), "{}", error(&installed));
    write_bucket(fixture.temp.path(), &repository.later);
    let update = ["update", "git-package", "--path", game];
    let plan = [
        "update",
        "git-package",
        "--path",
        game,
        "--dry-run",
        "--download",
    ];
    let prepared = run(&fixture, &repository.base, &plan);
    assert!(prepared.status.success(), "{}", error(&prepared));
    let archive = cache_archives(&fixture)
        .into_iter()
        .find(|path| path.to_string_lossy().contains(&repository.later))
        .expect("the prepared update caches the later archive");
    let file = archive.to_str().expect("archive path");
    let updated = run(
        &fixture,
        &repository.base,
        &[&update[..], &["--yes", "--file", file]].concat(),
    );
    assert!(updated.status.success(), "{}", error(&updated));
    assert_eq!(
        fs::read(installed_plugin(&fixture)).expect("updated plugin"),
        b"later-release"
    );
}
