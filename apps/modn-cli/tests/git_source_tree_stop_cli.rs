#[path = "support/catalog.rs"]
mod support;

#[path = "support/git_fixture.rs"]
mod git_fixture;

#[path = "support/git_source.rs"]
mod git_source;

#[path = "support/main_bucket.rs"]
mod main_bucket;

use git_fixture::repository;
use git_source::{add_stalled_then_serving_bucket, run_with_stub_git_env, stalling_git_program};
use support::{error, Fixture};

#[test]
fn an_unstoppable_git_tree_stops_the_install_before_the_next_source() {
    let fixture = Fixture::new();
    let repository = repository(fixture.temp.path());
    add_stalled_then_serving_bucket(&fixture, fixture.temp.path(), &repository.commit);
    fixture.register();
    let program = stalling_git_program(fixture.temp.path());

    let (output, _) = run_with_stub_git_env(
        &fixture,
        &repository.base,
        &[
            "install",
            "test-game/git-package",
            "--path",
            fixture.game.to_str().expect("game path"),
            "--yes",
        ],
        &program,
        "8000",
        &[("MODN_TEST_GIT_TREE_STOP_FAIL", "1")],
    );

    assert!(
        !output.status.success(),
        "an unstoppable fetch tree must stop the install: {}",
        error(&output)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("process tree could not be stopped"),
        "the error does not name the stopped tree: {stderr}"
    );
    assert!(
        !stderr
            .lines()
            .any(|line| line.contains("git fixture/project commit ")
                && line.trim_end().ends_with(": ok")),
        "the install tried the second source after an unstoppable tree: {stderr}"
    );
    assert!(
        !fixture.game.join("addons/git-source.plugin").exists(),
        "the second source installed a file after an unstoppable tree"
    );
}
