#![allow(
    dead_code,
    reason = "shared fixture exposes helpers for different test crates"
)]

//! Build `git` commands isolated from the caller's repository.
//!
//! A test process can run under a Git hook or another `git` command that
//! exports repository-location variables. Those variables override
//! `current_dir`, so every fixture command removes them first.

use std::process::Command;

/// Repository-location variables Git honours before it searches for a repository.
pub const REPOSITORY_VARIABLES: [&str; 9] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
    "GIT_PREFIX",
    "GIT_CEILING_DIRECTORIES",
];

/// Build a `git` command for a disposable fixture repository.
pub fn git_command() -> Command {
    let mut command = Command::new("git");
    for variable in REPOSITORY_VARIABLES {
        command.env_remove(variable);
    }
    command
}

#[test]
fn repository_variables_are_removed_from_fixture_git_commands() {
    let command = git_command();
    let removed: Vec<&str> = command
        .get_envs()
        .filter(|(_, value)| value.is_none())
        .map(|(name, _)| name.to_str().expect("variable name"))
        .collect();
    for variable in REPOSITORY_VARIABLES {
        assert!(
            removed.contains(&variable),
            "{variable} stays inherited by a fixture git command"
        );
    }
}
