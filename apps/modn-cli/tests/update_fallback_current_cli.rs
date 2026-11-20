#[path = "support/catalog.rs"]
mod support;

#[path = "support/git_fixture.rs"]
mod git_fixture;

#[path = "support/git_source.rs"]
mod git_source;

#[path = "support/main_bucket.rs"]
mod main_bucket;

use git_fixture::repository;
use git_source::{
    low_speed_git_program, run_with_stub_git_captured, write_two_source_bucket,
    write_two_source_manifest,
};
use serde_json::Value;
use support::{error, Fixture};

fn report(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("JSON report")
}

#[test]
fn a_fallback_install_from_the_second_git_source_is_current() {
    let fixture = Fixture::new();
    let repository = repository(fixture.temp.path());
    // The primary source pins a later commit; the fallback source serves the
    // pinned one. The install therefore records the fallback source.
    write_two_source_bucket(
        &fixture,
        fixture.temp.path(),
        "git-fallback-fixture",
        "low-speed",
        "1.0",
        &repository.later,
        &repository.commit,
    );
    fixture.register();
    let game = fixture.game.to_str().expect("game path");
    let program = low_speed_git_program(fixture.temp.path());

    let installed = run_with_stub_git_captured(
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
        &program,
        "30000",
    );
    assert!(installed.0.status.success(), "{}", error(&installed.0));

    let current = run_with_stub_git_captured(
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
        &program,
        "30000",
    );
    assert!(current.0.status.success(), "{}", error(&current.0));
    let current = report(&current.0);
    assert_eq!(
        current["up_to_date"], true,
        "the fallback release is not current: {current}"
    );
    assert_eq!(
        current["installed"]["source"]["repository"], "fixture/project",
        "the report drops the recorded serving source: {current}"
    );

    let updated = run_with_stub_git_captured(
        &fixture,
        &repository.base,
        &["--json", "update", "git-package", "--path", game, "--yes"],
        &program,
        "30000",
    );
    assert!(updated.0.status.success(), "{}", error(&updated.0));
    assert_eq!(
        report(&updated.0)["up_to_date"],
        true,
        "a real update follows its dry-run"
    );

    // The recorded fallback source now pins another commit: the release needs
    // an update even though the primary source did not change.
    write_two_source_manifest(
        fixture.temp.path(),
        "git-fallback-fixture",
        "low-speed",
        "1.0",
        &repository.later,
        &repository.later,
    );
    let changed = run_with_stub_git_captured(
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
        &program,
        "30000",
    );
    assert!(changed.0.status.success(), "{}", error(&changed.0));
    assert_eq!(
        report(&changed.0)["up_to_date"],
        false,
        "a changed fallback commit must need an update"
    );
}
