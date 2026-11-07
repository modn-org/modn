//! Hardened Git invocation shared by catalog sync and package sources.
//!
//! Git runs as an argv-only child process with a fixed configuration. No
//! system or user configuration, credential prompt, or repository hook can
//! change the transport or run commands.
use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::{Command, Output};

/// Fixed environment for one Git invocation.
pub(crate) struct GitEnvironment<'a> {
    /// Working directory; Git finds the repository from here.
    pub(crate) directory: &'a Path,
    /// Empty directory used as the hooks path.
    pub(crate) hooks: &'a Path,
    /// Permit the `file` transport. Only catalog sync and test builds do so.
    pub(crate) allow_file: bool,
}

/// Build a Git command with system and user configuration disabled.
pub(crate) fn command(environment: &GitEnvironment<'_>) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-c")
        .arg("protocol.allow=never")
        .arg("-c")
        .arg("protocol.https.allow=always")
        .arg("-c")
        .arg(format!("core.hooksPath={}", environment.hooks.display()))
        .arg("-c")
        .arg("credential.interactive=false");
    if environment.allow_file {
        command.arg("-c").arg("protocol.file.allow=always");
    }
    command
        .current_dir(environment.directory)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env_remove("GIT_ASKPASS")
        .env_remove("SSH_ASKPASS");
    command
}

fn run_output(command: &mut Command) -> Result<Output> {
    let output = command
        .output()
        .context("cannot run Git; install Git and ensure it is on PATH")?;
    if !output.status.success() {
        bail!(
            "Git operation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(output)
}

/// Run one Git command and return trimmed standard output.
pub(crate) fn run(environment: &GitEnvironment<'_>, arguments: &[&str]) -> Result<String> {
    let mut command = command(environment);
    command.args(arguments);
    let output = run_output(&mut command)?;
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

/// Run one Git command and return raw standard output.
pub(crate) fn run_bytes(environment: &GitEnvironment<'_>, arguments: &[&str]) -> Result<Vec<u8>> {
    let mut command = command(environment);
    command.args(arguments);
    let output = run_output(&mut command)?;
    Ok(output.stdout)
}

/// Create a private empty hooks directory for one Git invocation.
pub(crate) fn empty_hooks() -> Result<tempfile::TempDir> {
    tempfile::Builder::new()
        .prefix("modn-git-hooks-")
        .tempdir()
        .context("cannot create empty Git hooks directory")
}
