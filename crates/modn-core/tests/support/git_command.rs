#![allow(
    dead_code,
    reason = "shared fixture exposes helpers for different test crates"
)]

use std::process::Command;

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
