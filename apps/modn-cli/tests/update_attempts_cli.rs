#[path = "support/catalog.rs"]
mod support;

#[path = "support/git_fixture.rs"]
mod git_fixture;

#[path = "support/git_source.rs"]
mod git_source;

#[path = "support/git_source_helpers.rs"]
mod helpers;

#[path = "support/main_bucket.rs"]
mod main_bucket;

use git_fixture::repository;
use git_source::{low_speed_git_program, run_with_stub_git_captured, write_two_source_manifest};
use helpers::{add_bucket, run};
use serde_json::Value;
use support::{error, Fixture};

fn report(output: &std::process::Output) -> Value {
    assert!(output.status.success(), "{}", error(output));
    serde_json::from_slice(&output.stdout).expect("JSON report")
}

#[test]
fn update_json_dry_run_download_reports_every_source_attempt() {
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
    write_two_source_manifest(
        fixture.temp.path(),
        "git-fixture",
        "low-speed",
        "2.0",
        &repository.later,
        &repository.later,
    );
    let program = low_speed_git_program(fixture.temp.path());

    let preview = run_with_stub_git_captured(
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
            "--no-cache",
        ],
        &program,
        "30000",
    );
    let preview = report(&preview.0);
    let downloads = preview["downloads"]
        .as_array()
        .expect("the dry-run report lists the source attempts");
    assert_eq!(downloads.len(), 2, "{preview}");
    assert!(
        downloads[0]["source"]
            .as_str()
            .is_some_and(|source| source.contains("fixture/low-speed")),
        "the failed attempt is not first: {preview}"
    );
    assert_eq!(downloads[0]["outcome"], "failed", "{preview}");
    assert_eq!(downloads[0]["attempt"], 1, "{preview}");
    assert!(
        downloads[1]["source"]
            .as_str()
            .is_some_and(|source| source.contains("fixture/project")),
        "the serving attempt is not second: {preview}"
    );
    assert_eq!(downloads[1]["outcome"], "ok", "{preview}");

    let text = run_with_stub_git_captured(
        &fixture,
        &repository.base,
        &[
            "update",
            "git-package",
            "--path",
            game,
            "--dry-run",
            "--download",
            "--no-cache",
        ],
        &program,
        "30000",
    );
    assert!(text.0.status.success(), "{}", error(&text.0));
    let stdout = String::from_utf8_lossy(&text.0.stdout);
    assert!(
        stdout.lines().any(
            |line| line.starts_with("DOWNLOAD git fixture/low-speed commit ")
                && line.contains("attempt 1: timeout")
        ),
        "the text report drops the failed attempt: {stdout}"
    );
    assert!(
        stdout.lines().any(
            |line| line.starts_with("DOWNLOAD git fixture/project commit ")
                && line.trim_end().ends_with(": ok")
        ),
        "the text report drops the serving attempt: {stdout}"
    );
}
