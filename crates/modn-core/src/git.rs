//! Hardened Git invocation shared by catalog sync and package sources.
//!
//! Git runs as an argv-only child process with a fixed configuration. No
//! system or user configuration, credential prompt, or repository hook can
//! change the transport or run commands.
use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::{Command, Output};

use crate::acquisition::{CONNECT_TIMEOUT, READ_INACTIVITY_TIMEOUT};

/// Lowest transfer rate, in bytes per second, that Git accepts before it
/// aborts an HTTP or HTTPS transfer after the low-speed time.
const LOW_SPEED_LIMIT: u64 = 1;

/// The shared low-speed time of every Git transfer: the documented inactivity
/// limit of every source.
pub(crate) fn default_low_speed_time_seconds() -> u64 {
    (CONNECT_TIMEOUT + READ_INACTIVITY_TIMEOUT).as_secs()
}

/// Seconds that a Git HTTP or HTTPS transfer may stay below [`LOW_SPEED_LIMIT`].
///
/// A test build may shorten the time through `MODN_TEST_GIT_FETCH_IDLE_MS`, in
/// milliseconds, because Git accepts whole seconds or zero to disable the
/// limit. Release builds always use the documented inactivity limit.
pub(crate) fn low_speed_time_seconds() -> u64 {
    #[cfg(any(test, feature = "test-instruction-budget"))]
    if let Some(millis) = std::env::var("MODN_TEST_GIT_FETCH_IDLE_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
    {
        return (millis / 1000).max(1);
    }
    default_low_speed_time_seconds()
}

/// Fixed environment for one Git invocation.
pub(crate) struct GitEnvironment<'a> {
    /// Working directory; Git finds the repository from here.
    pub(crate) directory: &'a Path,
    /// Empty directory used as the hooks path.
    pub(crate) hooks: &'a Path,
    /// Permit the `file` transport. Only catalog sync and test builds do so.
    pub(crate) allow_file: bool,
}

// The executable that test code substitutes for Git on this thread. Tests
// share one process across parallel threads, so an in-process override stays
// thread-local and cannot leak into a command that another test runs.
#[cfg(test)]
thread_local! {
    pub(crate) static TEST_PROGRAM: std::cell::RefCell<Option<std::ffi::OsString>> =
        const { std::cell::RefCell::new(None) };
}

/// The Git program; a test build may replace it with a stub.
fn program() -> std::ffi::OsString {
    #[cfg(test)]
    if let Some(program) = TEST_PROGRAM.with(|slot| slot.borrow().clone()) {
        return program;
    }
    #[cfg(any(test, feature = "test-instruction-budget"))]
    if let Some(program) =
        std::env::var_os("MODN_TEST_GIT_FETCH_PROGRAM").filter(|value| !value.is_empty())
    {
        return program;
    }
    "git".into()
}

/// Build a Git command with system and user configuration disabled.
///
/// Every command, including fetch and clone, carries the shared HTTP low-speed
/// limits. Git and libcurl therefore end a stalled HTTP or HTTPS transfer
/// without Modn tracking or stopping the child process.
pub(crate) fn command(environment: &GitEnvironment<'_>) -> Command {
    command_with_low_speed_time(environment, low_speed_time_seconds())
}

/// Build a Git command with an explicit low-speed time, for tests.
pub(crate) fn command_with_low_speed_time(
    environment: &GitEnvironment<'_>,
    low_speed_seconds: u64,
) -> Command {
    let mut command = Command::new(program());
    command
        .arg("-c")
        .arg("protocol.allow=never")
        .arg("-c")
        .arg("protocol.https.allow=always")
        .arg("-c")
        .arg(format!("http.lowSpeedLimit={LOW_SPEED_LIMIT}"))
        .arg("-c")
        .arg(format!("http.lowSpeedTime={low_speed_seconds}"))
        .arg("-c")
        .arg(format!("core.hooksPath={}", environment.hooks.display()))
        .arg("-c")
        .arg("credential.interactive=false");
    if environment.allow_file {
        command.arg("-c").arg("protocol.file.allow=always");
    }
    #[cfg(any(test, feature = "test-instruction-budget"))]
    command.arg("-c").arg("protocol.http.allow=always");
    command
        .current_dir(environment.directory)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
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
        .env_remove("SSH_ASKPASS")
        .env_remove("GIT_HTTP_LOW_SPEED_LIMIT")
        .env_remove("GIT_HTTP_LOW_SPEED_TIME");
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn command_clears_the_low_speed_environment_overrides() {
        let directory = tempfile::tempdir().expect("directory");
        let hooks = empty_hooks().expect("hooks");
        let git = command(&GitEnvironment {
            directory: directory.path(),
            hooks: hooks.path(),
            allow_file: false,
        });
        for name in ["GIT_HTTP_LOW_SPEED_LIMIT", "GIT_HTTP_LOW_SPEED_TIME"] {
            let cleared = git
                .get_envs()
                .any(|(key, value)| key == OsStr::new(name) && value.is_none());
            assert!(cleared, "{name} must be cleared from the child environment");
        }
    }
}
