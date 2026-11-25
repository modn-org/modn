use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::{Command, Output};

use crate::acquisition::{CONNECT_TIMEOUT, READ_INACTIVITY_TIMEOUT};

/// Lowest transfer rate, in bytes per second, that Git accepts before it
/// aborts an HTTP or HTTPS transfer after the low-speed time.
const LOW_SPEED_LIMIT: u64 = 1;

pub(crate) fn default_low_speed_time_seconds() -> u64 {
    (CONNECT_TIMEOUT + READ_INACTIVITY_TIMEOUT).as_secs()
}

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

pub(crate) struct GitEnvironment<'a> {
    pub(crate) directory: &'a Path,
    pub(crate) hooks: &'a Path,
    pub(crate) allow_file: bool,
}

#[cfg(test)]
thread_local! {
    pub(crate) static TEST_PROGRAM: std::cell::RefCell<Option<std::ffi::OsString>> =
        const { std::cell::RefCell::new(None) };
}

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

pub(crate) fn command(environment: &GitEnvironment<'_>) -> Command {
    command_with_low_speed_time(environment, low_speed_time_seconds())
}

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

pub(crate) fn run(environment: &GitEnvironment<'_>, arguments: &[&str]) -> Result<String> {
    let mut command = command(environment);
    command.args(arguments);
    let output = run_output(&mut command)?;
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

pub(crate) fn run_bytes(environment: &GitEnvironment<'_>, arguments: &[&str]) -> Result<Vec<u8>> {
    let mut command = command(environment);
    command.args(arguments);
    let output = run_output(&mut command)?;
    Ok(output.stdout)
}

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
